#![allow(clippy::empty_docs)]
mod core;
mod nodes;
mod objects;
#[cfg(feature = "wayland")]
mod wayland;

use crate::core::client::CLIENTS;
use crate::core::destroy_queue;
use crate::nodes::items::camera;
use crate::nodes::{audio, drawable, hmd, input};
use crate::objects::input::eye_pointer::EyePointer;
use crate::objects::input::mouse_pointer::MousePointer;
use crate::objects::input::sk_controller::SkController;
use crate::objects::input::sk_hand::SkHand;
use crate::objects::play_space::PlaySpace;

use self::core::eventloop::EventLoop;
use clap::Parser;
use core::client_state::ClientStateParsed;
use directories::ProjectDirs;
use once_cell::sync::OnceCell;
use stardust_xr::server;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;
use stereokit_rust::material::Material;
use stereokit_rust::shader::Shader;
use stereokit_rust::sk::{
	sk_quit, AppMode, DepthMode, DisplayBlend, DisplayMode, QuitReason, SkSettings,
};
use stereokit_rust::system::{Handed, LogLevel, Renderer, World};
use stereokit_rust::tex::{SHCubemap, Tex, TexFormat, TexType};
use stereokit_rust::ui::Ui;
use stereokit_rust::util::{Color128, Device, Time};
use tokio::sync::Notify;
use tokio::task::LocalSet;
use tokio::{runtime::Handle, sync::oneshot};
use tracing::metadata::LevelFilter;
use tracing::{debug_span, error, info};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
struct CliArgs {
	/// Force flatscreen mode and use the mouse pointer as a 3D pointer
	#[clap(short, long, action)]
	flatscreen: bool,

	/// Run Stardust XR as an overlay with given priority
	#[clap(id = "PRIORITY", short = 'o', long = "overlay", action)]
	overlay_priority: Option<u32>,

	/// Run a script when ready for clients to connect. If this is not set the script at $HOME/.config/stardust/startup will be ran if it exists.
	#[clap(id = "PATH", short = 'e', long = "execute-startup-script", action)]
	startup_script: Option<PathBuf>,

	/// Restore the session with the given ID (or `latest`), ignoring the startup script. Sessions are stored in directories at `~/.local/state/stardust/`.
	#[clap(id = "SESSION_ID", long = "restore", action)]
	restore: Option<String>,
}

static STARDUST_INSTANCE: OnceCell<String> = OnceCell::new();
static STOP_NOTIFIER: Notify = Notify::const_new();

struct EventLoopInfo {
	tokio_handle: Handle,
	socket_path: PathBuf,
}

fn main() {
	let registry = tracing_subscriber::registry();

	#[cfg(feature = "profile_app")]
	let registry = registry.with(tracing_tracy::TracyLayer::new().with_filter(LevelFilter::DEBUG));

	#[cfg(feature = "profile_tokio")]
	let (console_layer, _) = console_subscriber::ConsoleLayer::builder().build();
	#[cfg(feature = "profile_tokio")]
	let registry = registry.with(console_layer);

	let log_layer = fmt::Layer::new()
		.with_thread_names(true)
		.with_ansi(true)
		.with_line_number(true)
		.with_filter(EnvFilter::from_default_env());
	registry.with(log_layer).init();

	let project_dirs = ProjectDirs::from("", "", "stardust");
	if project_dirs.is_none() {
		error!("Unable to get Stardust project directories, default skybox and startup script will not work.");
	}
	let cli_args = Arc::new(CliArgs::parse());

	let sk = SkSettings::default()
		.app_name("Stardust XR")
		.mode(if cli_args.flatscreen {
			AppMode::Simulator
		} else {
			AppMode::XR
		})
		.blend_preference(DisplayBlend::AnyTransparent)
		.depth_mode(DepthMode::D32)
		.log_filter(match EnvFilter::from_default_env().max_level_hint() {
			Some(LevelFilter::ERROR) => LogLevel::Error,
			Some(LevelFilter::WARN) => LogLevel::Warning,
			Some(LevelFilter::INFO) => LogLevel::Inform,
			Some(LevelFilter::DEBUG) => LogLevel::Diagnostic,
			Some(LevelFilter::TRACE) => LogLevel::Diagnostic,
			Some(LevelFilter::OFF) => LogLevel::None,
			None => LogLevel::Warning,
		})
		.overlay_app(cli_args.overlay_priority.is_some())
		.overlay_priority(cli_args.overlay_priority.unwrap_or(u32::MAX))
		.disable_desktop_input_window(true)
		.render_scaling(2.0)
		.init()
		.expect("StereoKit failed to initialize");
	info!("Init StereoKit");

	Renderer::multisample(0);
	Material::default().shader(Shader::pbr_clip());
	Ui::enable_far_interact(false);

	// Skytex/light stuff
	{
		if let Some(sky) = project_dirs
			.as_ref()
			.map(|dirs| dirs.config_dir().join("skytex.hdr"))
			.filter(|f| f.exists())
			.and_then(|p| SHCubemap::from_cubemap_equirectangular(p, true, 100).ok())
		{
			sky.render_as_sky();
		} else {
			Renderer::skytex(Tex::gen_color(
				Color128::BLACK,
				1,
				1,
				TexType::Cubemap,
				TexFormat::RGBA32,
			));
		}
	}

	let mut mouse_pointer = cli_args
		.flatscreen
		.then(MousePointer::new)
		.transpose()
		.unwrap();
	let mut hands = (!cli_args.flatscreen)
		.then(|| {
			let left = SkHand::new(Handed::Left).ok();
			let right = SkHand::new(Handed::Right).ok();
			left.zip(right)
		})
		.flatten();
	let mut controllers = (!cli_args.flatscreen)
		.then(|| {
			let left = SkController::new(Handed::Left).ok();
			let right = SkController::new(Handed::Right).ok();
			left.zip(right)
		})
		.flatten();
	let eye_pointer = (sk.get_active_display_mode() == DisplayMode::MixedReality
		&& Device::has_eye_gaze())
	.then(EyePointer::new)
	.transpose()
	.unwrap();

	let play_space = World::has_bounds().then(|| PlaySpace::new().ok()).flatten();

	let (info_sender, info_receiver) = oneshot::channel::<EventLoopInfo>();
	let event_thread = std::thread::Builder::new()
		.name("event_loop".to_owned())
		.spawn({
			let project_dirs = project_dirs.clone();
			move || event_loop(info_sender, project_dirs.clone())
		})
		.unwrap();
	let event_loop_info = info_receiver.blocking_recv().unwrap();
	let _tokio_handle = event_loop_info.tokio_handle.enter();

	#[cfg(feature = "wayland")]
	let mut wayland = wayland::Wayland::new().expect("Could not initialize wayland");
	info!("Stardust ready!");

	let mut startup_children = project_dirs
		.as_ref()
		.map(|project_dirs| {
			launch_start(
				&cli_args,
				project_dirs,
				&event_loop_info,
				#[cfg(feature = "wayland")]
				&wayland,
			)
		})
		.unwrap_or_default();

	let mut last_frame_delta = Duration::ZERO;
	let mut sleep_duration = Duration::ZERO;
	debug_span!("StereoKit").in_scope(|| {
		while let Some(token) = sk.step() {
			let _span = debug_span!("StereoKit step");
			let _span = _span.enter();

			hmd::frame();
			camera::update(token);
			#[cfg(feature = "wayland")]
			wayland.frame_event();
			destroy_queue::clear();

			if let Some(mouse_pointer) = &mut mouse_pointer {
				mouse_pointer.update();
			}
			if let Some((left_hand, right_hand)) = &mut hands {
				left_hand.update(&sk, token);
				right_hand.update(&sk, token);
			}
			if let Some((left_controller, right_controller)) = &mut controllers {
				left_controller.update(token);
				right_controller.update(token);
			}
			if let Some(eye_pointer) = &eye_pointer {
				eye_pointer.update();
			}
			if let Some(play_space) = &play_space {
				play_space.update();
			}
			input::process_input();
			nodes::root::Root::send_frame_events(Time::get_step_unscaled());
			adaptive_sleep(
				&mut last_frame_delta,
				&mut sleep_duration,
				Duration::from_micros(250),
			);

			#[cfg(feature = "wayland")]
			wayland.update();
			drawable::draw(token);
			audio::update();
			#[cfg(feature = "wayland")]
			wayland.make_context_current();
		}
	});

	info!("Cleanly shut down StereoKit");
	#[cfg(feature = "wayland")]
	drop(wayland);

	STOP_NOTIFIER.notify_waiters();
	event_thread
		.join()
		.expect("Failed to cleanly shut down event loop")
		.unwrap();
	for mut startup_child in startup_children.drain(..) {
		let _ = startup_child.kill();
	}

	info!("Cleanly shut down Stardust");
}

fn adaptive_sleep(
	last_frame_delta: &mut Duration,
	sleep_duration: &mut Duration,
	sleep_duration_increase: Duration,
) {
	let frame_delta = Duration::from_secs_f64(Time::get_step_unscaled());
	if *last_frame_delta < frame_delta {
		if let Some(frame_delta_delta) = frame_delta.checked_sub(*last_frame_delta) {
			if let Some(new_sleep_duration) = sleep_duration.checked_sub(frame_delta_delta) {
				*sleep_duration = new_sleep_duration;
			}
		}
	} else {
		*sleep_duration += sleep_duration_increase;
	}

	debug_span!("Sleep", ?sleep_duration, ?frame_delta, ?last_frame_delta).in_scope(|| {
		*last_frame_delta = frame_delta;
		std::thread::sleep(*sleep_duration); // to give clients a chance to even update anything before drawing
	});
}

// #[tokio::main]
#[tokio::main(flavor = "current_thread")]
async fn event_loop(
	info_sender: oneshot::Sender<EventLoopInfo>,
	project_dirs: Option<ProjectDirs>,
) -> color_eyre::eyre::Result<()> {
	let socket_path =
		server::get_free_socket_path().expect("Unable to find a free stardust socket path");
	STARDUST_INSTANCE.set(socket_path.file_name().unwrap().to_string_lossy().into_owned()).expect("Someone hasn't done their job, yell at Nova because how is this set multiple times what the hell");
	let _event_loop = EventLoop::new(socket_path.clone()).expect("Couldn't create server socket");
	info!("Init event loop");
	info!(
		socket_path = ?socket_path.display(),
		"Stardust socket created"
	);
	let _ = info_sender.send(EventLoopInfo {
		tokio_handle: Handle::current(),
		socket_path,
	});

	STOP_NOTIFIER.notified().await;
	println!("Stopping...");
	if let Some(project_dirs) = project_dirs {
		save_session(&project_dirs).await;
	}

	info!("Cleanly shut down event loop");

	unsafe {
		sk_quit(QuitReason::SystemClose);
	}

	Ok(())
}

fn launch_start(
	cli_args: &CliArgs,
	project_dirs: &ProjectDirs,
	event_loop_info: &EventLoopInfo,
	#[cfg(feature = "wayland")] wayland: &wayland::Wayland,
) -> Vec<Child> {
	if let Some(session_id) = &cli_args.restore {
		let session_dir = project_dirs.state_dir().unwrap().join(session_id);
		return restore_session(&session_dir, event_loop_info, wayland);
	}
	let startup_script_path = cli_args
		.startup_script
		.clone()
		.and_then(|p| p.canonicalize().ok())
		.unwrap_or_else(|| project_dirs.config_dir().join("startup"));
	run_script(&startup_script_path, event_loop_info, wayland)
}

fn restore_session(
	session_dir: &Path,
	event_loop_info: &EventLoopInfo,
	#[cfg(feature = "wayland")] wayland: &wayland::Wayland,
) -> Vec<Child> {
	let Ok(clients) = session_dir.read_dir() else {
		return Vec::new();
	};
	clients
		.filter_map(Result::ok)
		.filter_map(|c| ClientStateParsed::from_file(&c.path()))
		.filter_map(ClientStateParsed::launch_command)
		.filter_map(|startup_command| {
			run_client(
				startup_command,
				event_loop_info,
				#[cfg(feature = "wayland")]
				wayland,
			)
		})
		.collect()
}

fn run_script(
	script_path: &Path,
	event_loop_info: &EventLoopInfo,
	#[cfg(feature = "wayland")] wayland: &wayland::Wayland,
) -> Vec<Child> {
	let _ = std::fs::set_permissions(script_path, std::fs::Permissions::from_mode(0o755));
	let startup_command = Command::new(script_path);
	run_client(
		startup_command,
		event_loop_info,
		#[cfg(feature = "wayland")]
		wayland,
	)
	.map(|c| vec![c])
	.unwrap_or_default()
}

fn run_client(
	mut command: Command,
	event_loop_info: &EventLoopInfo,
	#[cfg(feature = "wayland")] wayland: &wayland::Wayland,
) -> Option<Child> {
	command.stdin(Stdio::null());
	command.stdout(Stdio::null());
	command.stderr(Stdio::null());
	command.env(
		"FLAT_WAYLAND_DISPLAY",
		std::env::var_os("WAYLAND_DISPLAY").unwrap_or_default(),
	);
	command.env(
		"STARDUST_INSTANCE",
		event_loop_info
			.socket_path
			.file_name()
			.expect("Stardust socket path not found"),
	);
	#[cfg(feature = "wayland")]
	{
		if let Some(wayland_socket) = wayland.socket_name.as_ref() {
			command.env("WAYLAND_DISPLAY", wayland_socket);
		}
		command.env("GDK_BACKEND", "wayland");
		command.env("QT_QPA_PLATFORM", "wayland");
		command.env("MOZ_ENABLE_WAYLAND", "1");
		command.env("CLUTTER_BACKEND", "wayland");
		command.env("SDL_VIDEODRIVER", "wayland");
	}
	let child = command.spawn().ok()?;
	Some(child)
}

async fn save_session(project_dirs: &ProjectDirs) {
	let session_id = nanoid::nanoid!();
	let state_dir = project_dirs.state_dir().unwrap();
	let session_dir = state_dir.join(&session_id);
	std::fs::create_dir_all(&session_dir).unwrap();
	let _ = std::fs::remove_dir_all(state_dir.join("latest"));
	std::os::unix::fs::symlink(&session_dir, state_dir.join("latest")).unwrap();

	let local_set = LocalSet::new();
	for client in CLIENTS.get_vec() {
		let session_dir = session_dir.clone();
		local_set.spawn_local(async move {
			tokio::select! {
				biased;
				s = client.save_state() => {if let Some(s) = s { s.to_file(&session_dir) }},
				_ = tokio::time::sleep(Duration::from_millis(100)) => (),
			}
		});
	}
	local_set.await;
	println!("Session ID for restore is {session_id}");
}

mod core;
mod nodes;
mod objects;
#[cfg(feature = "wayland")]
mod wayland;

use crate::core::destroy_queue;
use crate::nodes::{audio, drawable, hmd, input};
use crate::objects::input::mouse_pointer::MousePointer;
use crate::objects::input::sk_controller::SkController;
use crate::objects::input::sk_hand::SkHand;

use self::core::eventloop::EventLoop;
use clap::Parser;
use color_eyre::eyre::Result;
use directories::ProjectDirs;
use once_cell::sync::OnceCell;
use stardust_xr::server;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use stereokit::DisplayBlend;
use stereokit::{
	named_colors::BLACK, DepthMode, DisplayMode, Handed, LogLevel, StereoKitMultiThread,
	TextureFormat, TextureType,
};
use tokio::{runtime::Handle, sync::oneshot};
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

	/// Don't create a tip input for controller because SOME RUNTIMES will lie
	#[clap(long, action)]
	disable_controller: bool,

	/// Run a script when ready for clients to connect. If this is not set the script at $HOME/.config/stardust/startup will be ran if it exists.
	#[clap(id = "PATH", short = 'e', long = "execute-startup-script", action)]
	startup_script: Option<PathBuf>,
}

static STARDUST_INSTANCE: OnceCell<String> = OnceCell::new();

struct EventLoopInfo {
	tokio_handle: Handle,
	socket_path: PathBuf,
}

fn main() -> Result<()> {
	let registry = tracing_subscriber::registry();
	#[cfg(feature = "profile_app")]
	let (chrome_layer, _guard) = tracing_chrome::ChromeLayerBuilder::new()
		.include_args(true)
		.build();
	#[cfg(feature = "profile_app")]
	let registry = registry.with(chrome_layer);

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

	let sk = stereokit::Settings {
		app_name: "Stardust XR".to_string(),
		display_preference: if cli_args.flatscreen {
			DisplayMode::Flatscreen
		} else {
			DisplayMode::MixedReality
		},
		blend_preference: DisplayBlend::AnyTransparent,
		depth_mode: DepthMode::D32,
		log_filter: LogLevel::None,
		overlay_app: cli_args.overlay_priority.is_some(),
		overlay_priority: cli_args.overlay_priority.unwrap_or(u32::MAX),
		disable_desktop_input_window: true,
		..Default::default()
	}
	.init()
	.expect("StereoKit failed to initialize");
	info!("Init StereoKit");

	sk.material_set_shader(
		sk.material_find("default/material_pbr")?,
		sk.shader_find("default/shader_pbr_clip")?,
	);

	// Skytex/light stuff
	{
		if let Some((light, tex)) = project_dirs
			.as_ref()
			.and_then(|dirs| {
				let skytex_path = dirs.config_dir().join("skytex.hdr");
				skytex_path
					.exists()
					.then(|| sk.tex_create_cubemap_file(&skytex_path, true, 100).ok())
			})
			.flatten()
		{
			sk.render_set_skytex(&tex);
			sk.render_set_skylight(light);
		} else {
			sk.render_set_skytex(sk.tex_gen_color(
				BLACK,
				1,
				1,
				TextureType::CUBEMAP,
				TextureFormat::RGBA32,
			));
		}
	}

	let mouse_pointer = cli_args.flatscreen.then(MousePointer::new).transpose()?;
	let mut hands = (!cli_args.flatscreen)
		.then(|| {
			let left = SkHand::new(Handed::Left).ok();
			let right = SkHand::new(Handed::Right).ok();
			left.zip(right)
		})
		.flatten();
	let mut controllers = (!cli_args.flatscreen && !cli_args.disable_controller)
		.then(|| {
			let left = SkController::new(Handed::Left).ok();
			let right = SkController::new(Handed::Right).ok();
			left.zip(right)
		})
		.flatten();

	if hands.is_none() {
		// unsafe {
		// 	stereokit::sys::input_hand_visible(stereokit::sys::handed__handed_left, false as i32);
		// 	stereokit::sys::input_hand_visible(stereokit::sys::handed__handed_right, false as i32);
		// }
	}

	let (event_stop_tx, event_stop_rx) = oneshot::channel::<()>();
	let (info_sender, info_receiver) = oneshot::channel::<EventLoopInfo>();
	let event_thread = std::thread::Builder::new()
		.name("event_loop".to_owned())
		.spawn(move || event_loop(info_sender, event_stop_rx))?;
	let event_loop_info = info_receiver.blocking_recv()?;
	let _tokio_handle = event_loop_info.tokio_handle.enter();

	#[cfg(feature = "wayland")]
	let mut wayland = wayland::Wayland::new()?;
	info!("Stardust ready!");

	if let Some(project_dirs) = project_dirs.as_ref() {
		let startup_script_path = cli_args
			.startup_script
			.clone()
			.and_then(|p| p.canonicalize().ok())
			.unwrap_or_else(|| project_dirs.config_dir().join("startup"));
		let _startup = Command::new(startup_script_path)
			.env("WAYLAND_DISPLAY", &wayland.socket_name)
			.env(
				"STARDUST_INSTANCE",
				event_loop_info
					.socket_path
					.file_name()
					.expect("Stardust socket path not found"),
			)
			.spawn();
	}

	let mut last_frame_delta = Duration::ZERO;
	let mut sleep_duration = Duration::ZERO;
	debug_span!("StereoKit").in_scope(|| {
		sk.run(
			|sk| {
				let _span = debug_span!("StereoKit step");
				let _span = _span.enter();

				hmd::frame(sk);
				#[cfg(feature = "wayland")]
				wayland.frame_event(sk);
				destroy_queue::clear();

				if let Some(mouse_pointer) = &mouse_pointer {
					mouse_pointer.update(sk);
				}
				if let Some((left_hand, right_hand)) = &mut hands {
					left_hand.update(sk);
					right_hand.update(sk);
				}
				if let Some((left_controller, right_controller)) = &mut controllers {
					left_controller.update(sk);
					right_controller.update(sk);
				}
				input::process_input();
				nodes::root::Root::send_frame_events(sk.time_elapsed_unscaled());
				{
					let frame_delta = Duration::from_secs_f64(sk.time_elapsed_unscaled());
					if last_frame_delta < frame_delta {
						if let Some(frame_delta_delta) = frame_delta.checked_sub(last_frame_delta) {
							if let Some(new_sleep_duration) =
								sleep_duration.checked_sub(frame_delta_delta)
							{
								sleep_duration = new_sleep_duration;
							}
						}
					} else {
						sleep_duration += Duration::from_micros(250);
					}

					debug_span!("Sleep", ?sleep_duration, ?frame_delta, ?last_frame_delta)
						.in_scope(|| {
							last_frame_delta = frame_delta;
							std::thread::sleep(sleep_duration); // to give clients a chance to even update anything before drawing
						});
				}
				#[cfg(feature = "wayland")]
				wayland.update(sk);
				drawable::draw(sk);
				audio::update(sk);
				#[cfg(feature = "wayland")]
				wayland.make_context_current();
			},
			|_| {
				info!("Cleanly shut down StereoKit");
			},
		)
	});

	#[cfg(feature = "wayland")]
	drop(wayland);

	let _ = event_stop_tx.send(());
	event_thread
		.join()
		.expect("Failed to cleanly shut down event loop")?;
	info!("Cleanly shut down Stardust");
	Ok(())
}

#[tokio::main]
// #[tokio::main(flavor = "current_thread")]
async fn event_loop(
	info_sender: oneshot::Sender<EventLoopInfo>,
	stop_rx: oneshot::Receiver<()>,
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

	let result = tokio::select! {
		biased;
		_ = tokio::signal::ctrl_c() => Ok(()),
		_ = stop_rx => Ok(()),
	};

	info!("Cleanly shut down event loop");

	unsafe {
		stereokit::sys::sk_quit();
	}

	result
}

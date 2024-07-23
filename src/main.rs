#![allow(clippy::empty_docs)]
mod core;
mod nodes;
mod objects;
mod session;
#[cfg(feature = "wayland")]
mod wayland;

use crate::core::destroy_queue;
use crate::nodes::items::camera;
use crate::nodes::{audio, drawable, input};

use clap::Parser;
use core::client::Client;
use core::task;
use directories::ProjectDirs;
use objects::ServerObjects;
use once_cell::sync::OnceCell;
use session::{launch_start, save_session};
use stardust_xr::server;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use stereokit_rust::material::Material;
use stereokit_rust::shader::Shader;
use stereokit_rust::sk::{sk_quit, AppMode, DepthMode, OriginMode, QuitReason, SkSettings};
use stereokit_rust::system::{LogLevel, Renderer};
use stereokit_rust::tex::{SHCubemap, Tex, TexFormat, TexType};
use stereokit_rust::ui::Ui;
use stereokit_rust::util::{Color128, Time};
use tokio::net::UnixListener;
use tokio::sync::Notify;
use tracing::metadata::LevelFilter;
use tracing::{debug_span, error, info};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use zbus::fdo::ObjectManager;
use zbus::Connection;

#[derive(Debug, Clone, Parser)]
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

// #[tokio::main]
#[tokio::main(flavor = "current_thread")]
async fn main() {
	color_eyre::install().unwrap();

	let registry = tracing_subscriber::registry();

	#[cfg(feature = "profile_app")]
	let registry = registry.with(
		tracing_tracy::TracyLayer::new(tracing_tracy::DefaultConfig::default())
			.with_filter(LevelFilter::DEBUG),
	);

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

	let cli_args = CliArgs::parse();

	let socket_path =
		server::get_free_socket_path().expect("Unable to find a free stardust socket path");
	STARDUST_INSTANCE.set(socket_path.file_name().unwrap().to_string_lossy().into_owned()).expect("Someone hasn't done their job, yell at Nova because how is this set multiple times what the hell");
	info!(
		socket_path = ?socket_path.display(),
		"Stardust socket created"
	);
	let socket =
		UnixListener::bind(socket_path).expect("Couldn't spawn stardust server at {socket_path}");
	task::new(|| "client join loop", async move {
		loop {
			let Ok((stream, _)) = socket.accept().await else {
				continue;
			};
			if let Err(e) = Client::from_connection(stream) {
				error!(?e, "Unable to create client from connection");
			}
		}
	})
	.unwrap();
	info!("Init client join loop");

	let project_dirs = ProjectDirs::from("", "", "stardust");
	if project_dirs.is_none() {
		error!("Unable to get Stardust project directories, default skybox and startup script will not work.");
	}

	let dbus_connection = Connection::session().await.unwrap();
	dbus_connection
		.request_name("org.stardustxr.HMD")
		.await
		.expect("Another instance of the server is running. This is not supported currently (but is planned).");

	dbus_connection
		.object_server()
		.at("/", ObjectManager)
		.await
		.expect("Couldn't add the object manager");

	let sk_ready_notifier = Arc::new(Notify::new());
	let stereokit_loop = tokio::task::spawn_blocking({
		let sk_ready_notifier = sk_ready_notifier.clone();
		let project_dirs = project_dirs.clone();
		let cli_args = cli_args.clone();
		let dbus_connection = dbus_connection.clone();
		move || stereokit_loop(sk_ready_notifier, project_dirs, cli_args, dbus_connection)
	});
	sk_ready_notifier.notified().await;
	let mut startup_children = project_dirs
		.as_ref()
		.map(|project_dirs| launch_start(&cli_args, project_dirs))
		.unwrap_or_default();

	tokio::select! {
		_ = stereokit_loop => (),
		_ = tokio::signal::ctrl_c() => unsafe {sk_quit(QuitReason::SystemClose)},
	}
	info!("Stopping...");
	if let Some(project_dirs) = project_dirs {
		save_session(&project_dirs).await;
	}
	for mut startup_child in startup_children.drain(..) {
		let _ = startup_child.kill();
	}

	info!("Cleanly shut down Stardust");
}

fn stereokit_loop(
	sk_ready_notifier: Arc<Notify>,
	project_dirs: Option<ProjectDirs>,
	args: CliArgs,
	dbus_connection: Connection,
) {
	let sk = SkSettings::default()
		.app_name("Stardust XR")
		.mode(if args.flatscreen {
			AppMode::Simulator
		} else {
			AppMode::XR
		})
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
		.overlay_app(args.overlay_priority.is_some())
		.overlay_priority(args.overlay_priority.unwrap_or(u32::MAX))
		.disable_desktop_input_window(true)
		.origin(OriginMode::Local)
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

	#[cfg(feature = "wayland")]
	let mut wayland = wayland::Wayland::new().expect("Could not initialize wayland");
	#[cfg(feature = "wayland")]
	wayland.make_context_current();
	sk_ready_notifier.notify_waiters();
	info!("Stardust ready!");

	let mut objects = ServerObjects::new(dbus_connection.clone(), &sk);

	let mut last_frame_delta = Duration::ZERO;
	let mut sleep_duration = Duration::ZERO;
	debug_span!("StereoKit").in_scope(|| {
		while let Some(token) = sk.step() {
			let _span = debug_span!("StereoKit step");
			let _span = _span.enter();

			camera::update(token);
			#[cfg(feature = "wayland")]
			wayland.frame_event();
			destroy_queue::clear();

			objects.update(&sk, token);
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
		}
	});

	info!("Cleanly shut down StereoKit");
	#[cfg(feature = "wayland")]
	drop(wayland);
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

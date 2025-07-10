#![allow(clippy::empty_docs)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::type_complexity)]
mod core;
mod nodes;
mod objects;
mod session;
#[cfg(feature = "wayland")]
mod wayland;

use crate::core::destroy_queue;
use crate::nodes::input;

use bevy::MinimalPlugins;
use bevy::a11y::AccessibilityPlugin;
use bevy::app::{App, ScheduleRunnerPlugin, TerminalCtrlCHandlerPlugin};
use bevy::asset::{AssetMetaCheck, UnapprovedPathMode};
use bevy::audio::AudioPlugin;
use bevy::core_pipeline::CorePipelinePlugin;
use bevy::core_pipeline::oit::OrderIndependentTransparencySettings;
use bevy::diagnostic::DiagnosticsPlugin;
use bevy::ecs::schedule::{ExecutorKind, ScheduleLabel};
use bevy::gizmos::GizmoPlugin;
use bevy::gltf::GltfPlugin;
use bevy::input::InputPlugin;
use bevy::pbr::PbrPlugin;
use bevy::render::settings::{Backends, RenderCreation, WgpuSettings};
use bevy::render::{RenderDebugFlags, RenderPlugin};
use bevy::scene::ScenePlugin;
use bevy::winit::{WakeUp, WinitPlugin};
use bevy_mod_openxr::action_set_attaching::OxrActionAttachingPlugin;
use bevy_mod_openxr::action_set_syncing::OxrActionSyncingPlugin;
use bevy_mod_openxr::add_xr_plugins;
use bevy_mod_openxr::exts::OxrExtensions;
use bevy_mod_openxr::features::overlay::OxrOverlaySettings;
use bevy_mod_openxr::features::passthrough::OxrPassthroughPlugin;
use bevy_mod_openxr::init::{OxrInitPlugin, should_run_frame_loop};
use bevy_mod_openxr::reference_space::OxrReferenceSpacePlugin;
use bevy_mod_openxr::render::{OxrRenderPlugin, OxrWaitFrameSystem};
use bevy_mod_openxr::resources::{OxrFrameState, OxrFrameWaiter, OxrSessionConfig};
use bevy_mod_openxr::types::AppInfo;
use bevy_mod_xr::camera::XrProjection;
use bevy_mod_xr::session::{XrFirst, XrHandleEvents, XrSessionPlugin};
use clap::Parser;
use core::client::{Client, tick_internal_client};
use core::entity_handle::EntityHandlePlugin;
use core::graphics_info::GraphicsInfo;
use core::task;
use directories::ProjectDirs;
use nodes::audio::AudioNodePlugin;
use nodes::drawable::lines::LinesNodePlugin;
use nodes::drawable::model::ModelNodePlugin;
use nodes::drawable::text::TextNodePlugin;
use nodes::spatial::SpatialNodePlugin;
use objects::input::mouse_pointer::FlatscreenInputPlugin;
use objects::input::oxr_controller::ControllerPlugin;
use objects::input::oxr_hand::HandPlugin;
use objects::play_space::PlaySpacePlugin;
use openxr::{EnvironmentBlendMode, ReferenceSpaceType};
use session::{launch_start, save_session};
use stardust_xr::schemas::dbus::object_registry::ObjectRegistry;
use stardust_xr::server;
use std::ops::DerefMut as _;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use tokio::net::UnixListener;
use tokio::sync::Notify;
use tokio::task::JoinError;
use tracing::metadata::LevelFilter;
use tracing::{error, info};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};
use wayland::Wayland;
use zbus::Connection;
use zbus::fdo::ObjectManager;

use bevy::prelude::*;

#[derive(Debug, Clone, Parser)]
#[clap(author, version, about, long_about = None)]
struct CliArgs {
	/// Force flatscreen mode and use the mouse pointer as a 3D pointer
	#[clap(short, long, action)]
	flatscreen: bool,

	/// If monado insists on emulating them, set this flag...we want the raw input
	#[clap(long)]
	disable_controllers: bool,
	/// If monado insists on emulating , set this flag...we want the raw input
	#[clap(long)]
	disable_hands: bool,

	/// Run Stardust XR as an overlay with given priority
	#[clap(id = "PRIORITY", short = 'o', long = "overlay", action)]
	overlay_priority: Option<u32>,

	/// Debug the clients started by the server
	#[clap(short = 'd', long = "debug", action)]
	debug_launched_clients: bool,

	/// Run a script when ready for clients to connect. If this is not set the script at $HOME/.config/stardust/startup will be ran if it exists.
	#[clap(id = "PATH", short = 'e', long = "execute-startup-script", action)]
	startup_script: Option<PathBuf>,

	/// Restore the session with the given ID (or `latest`), ignoring the startup script. Sessions are stored in directories at `~/.local/state/stardust/`.
	#[clap(id = "SESSION_ID", long = "restore", action)]
	restore: Option<String>,
}

pub type BevyMaterial = StandardMaterial;

static STARDUST_INSTANCE: OnceLock<String> = OnceLock::new();

// #[tokio::main(flavor = "current_thread")]
#[tokio::main]
async fn main() -> Result<AppExit, JoinError> {
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
		.with_filter(
			EnvFilter::builder()
				.with_default_directive(LevelFilter::WARN.into())
				.from_env_lossy(),
		);
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
		error!(
			"Unable to get Stardust project directories, default skybox and startup script will not work."
		);
	}

	let dbus_connection = Connection::session()
		.await
		.expect("Could not open dbus session");
	dbus_connection
		.request_name("org.stardustxr.HMD")
		.await
		.expect(
			"Another instance of the server is running. This is not supported currently (but is planned).",
		);

	dbus_connection
		.object_server()
		.at("/", ObjectManager)
		.await
		.expect("Couldn't add the object manager");

	let object_registry = ObjectRegistry::new(&dbus_connection).await.expect(
		"Couldn't make the object registry to find all objects with given interfaces in d-bus",
	);

	let _wayland = Wayland::new(None).expect("Couldn't create Wayland instance");

	let ready_notifier = Arc::new(Notify::new());
	let io_loop = tokio::task::spawn_blocking({
		let ready_notifier = ready_notifier.clone();
		let project_dirs = project_dirs.clone();
		let cli_args = cli_args.clone();
		let dbus_connection = dbus_connection.clone();
		move || {
			bevy_loop(
				ready_notifier,
				project_dirs,
				cli_args,
				dbus_connection,
				object_registry,
			)
		}
	});
	ready_notifier.notified().await;
	let mut startup_children = project_dirs
		.as_ref()
		.map(|project_dirs| launch_start(&cli_args, project_dirs))
		.unwrap_or_default();
	let return_value = io_loop.await;
	info!("Stopping...");
	if let Some(project_dirs) = project_dirs {
		save_session(&project_dirs).await;
	}
	for mut startup_child in startup_children.drain(..) {
		let _ = startup_child.kill();
	}

	info!("Cleanly shut down Stardust");
	return_value
}

// static DEFAULT_SKYTEX: OnceLock<Tex> = OnceLock::new();
// static DEFAULT_SKYLIGHT: OnceLock<SphericalHarmonics> = OnceLock::new();

#[derive(ScheduleLabel, Hash, Debug, PartialEq, Eq, Clone, Copy)]
pub struct PreFrameWait;
#[derive(Resource, Deref)]
pub struct ObjectRegistryRes(ObjectRegistry);
#[derive(Resource, Deref)]
pub struct DbusConnection(Connection);

fn bevy_loop(
	ready_notifier: Arc<Notify>,
	_project_dirs: Option<ProjectDirs>,
	args: CliArgs,
	dbus_connection: Connection,
	object_registry: ObjectRegistry,
) -> AppExit {
	let mut app = App::new();
	app.insert_resource(DbusConnection(dbus_connection));
	app.add_plugins(AssetPlugin {
		meta_check: AssetMetaCheck::Never,
		unapproved_path_mode: UnapprovedPathMode::Allow,
		..default()
	});
	let mut plugins = MinimalPlugins
		.build()
		.add(DiagnosticsPlugin)
		.add(TransformPlugin)
		.add(InputPlugin)
		.add(AccessibilityPlugin);
	plugins = plugins
		.add(TerminalCtrlCHandlerPlugin)
		// bevy_mod_openxr will replace this, TODO: figure out how to mix this with
		// bevy-dmabuf
		.add(RenderPlugin {
			render_creation: RenderCreation::Automatic(WgpuSettings {
				backends: Some(Backends::VULKAN),
				..Default::default()
			}),
			..Default::default()
		})
		.add(ImagePlugin::default())
		.add(CorePipelinePlugin)
		// theoretically we shouldn't need this because of bevy_sk, but everything is tangled in
		// there and idk what we actually need to run
		.add(PbrPlugin {
			// this seems to only apply to StandardMaterial, we don't use that
			prepass_enabled: true,
			add_default_deferred_lighting_plugin: false,
			use_gpu_instance_buffer_builder: true,
			debug_flags: RenderDebugFlags::default(),
		})
		// required for gltf
		.add(ScenePlugin)
		.add(GltfPlugin::default())
		// .add(AnimationPlugin)
		.add(AudioPlugin::default())
		.add(GizmoPlugin)
		.add(WindowPlugin::default());
	let mut task_pool_plugin = TaskPoolPlugin::default();
	// make tokio work
	let handle = tokio::runtime::Handle::current();
	let enter_runtime_context = Arc::new(move || {
		// TODO: this might be a memory leak
		std::mem::forget(handle.enter());
	});
	task_pool_plugin.task_pool_options.io.on_thread_spawn = Some(enter_runtime_context.clone());
	task_pool_plugin.task_pool_options.compute.on_thread_spawn =
		Some(enter_runtime_context.clone());
	task_pool_plugin
		.task_pool_options
		.async_compute
		.on_thread_spawn = Some(enter_runtime_context.clone());
	plugins = plugins.set(task_pool_plugin);
	if args.flatscreen
		|| std::env::var_os("DISPLAY").is_some()
		|| std::env::var_os("WAYLAND_DISPLAY").is_some()
	{
		let mut plugin = WinitPlugin::<WakeUp>::default();
		plugin.run_on_any_thread = true;
		plugins = plugins
			.add(plugin)
			.disable::<ScheduleRunnerPlugin>()
			.add(FlatscreenInputPlugin);
	}
	app.add_plugins(if !args.flatscreen {
		add_xr_plugins(plugins)
			.set(OxrInitPlugin {
				app_info: AppInfo {
					name: "Stardust XR".into(),
					version: bevy_mod_openxr::types::Version(0, 44, 1),
				},
				exts: {
					// all OpenXR extensions can be requested here
					let mut exts = OxrExtensions::default();
					exts.enable_hand_tracking();
					if args.overlay_priority.is_some() {
						exts.enable_extx_overlay();
					}
					exts
				},
				..default()
			})
			.set(OxrRenderPlugin {
				default_wait_frame: false,
				..default()
			})
			.set(OxrReferenceSpacePlugin {
				default_primary_ref_space: ReferenceSpaceType::LOCAL,
			})
			// Disable a bunch of unneeded plugins
			// this plugin uses the fb extention, blend mode still works
			.disable::<OxrPassthroughPlugin>()
			// we don't do any action stuff that needs to integrate with the ecosystem
			.disable::<OxrActionAttachingPlugin>()
			.disable::<OxrActionSyncingPlugin>()
	} else {
		// enable a event
		plugins.add(XrSessionPlugin { auto_handle: false })
	});

	app.add_plugins(bevy_sk::hand::HandPlugin);
	// app.add_plugins(HandGizmosPlugin);
	app.world_mut().resource_mut::<AmbientLight>().brightness = 2000.0;
	if let Some(priority) = args.overlay_priority {
		app.insert_resource(OxrOverlaySettings {
			session_layer_placement: priority,
			..default()
		});
	}
	app.insert_resource(OxrSessionConfig {
		blend_modes: Some(vec![
			EnvironmentBlendMode::ALPHA_BLEND,
			EnvironmentBlendMode::ADDITIVE,
			EnvironmentBlendMode::OPAQUE,
		]),
		..default()
	});
	let mut pre_frame_wait = Schedule::new(PreFrameWait);
	pre_frame_wait.set_executor_kind(ExecutorKind::MultiThreaded);
	app.add_schedule(pre_frame_wait);
	app.insert_resource(ClearColor(Color::BLACK.with_alpha(0.0)));
	app.insert_resource(ObjectRegistryRes(object_registry));
	#[cfg(feature = "bevy_debugging")]
	{
		use bevy::remote::{RemotePlugin, http::RemoteHttpPlugin};
		app.add_plugins((RemotePlugin::default(), RemoteHttpPlugin::default()));
	}
	// the Stardust server plugins
	// infra plugins
	app.add_plugins(EntityHandlePlugin);
	// node plugins
	app.add_plugins((
		SpatialNodePlugin,
		ModelNodePlugin,
		TextNodePlugin,
		LinesNodePlugin,
		AudioNodePlugin,
	));
	// object plugins
	app.add_plugins((PlaySpacePlugin, HandPlugin, ControllerPlugin));
	app.add_systems(PostStartup, move || {
		ready_notifier.notify_waiters();
	});
	app.add_systems(Update, (add_oit, update_cameras));
	app.add_systems(
		XrFirst,
		xr_step
			.in_set(OxrWaitFrameSystem)
			.in_set(XrHandleEvents::FrameLoop),
	);

	app.run()
}
fn update_cameras(mut camera: Query<&mut Projection, (With<Camera3d>,)>) {
	for mut projection in &mut camera {
		match projection.deref_mut() {
			Projection::Perspective(perspective_projection) => perspective_projection.near = 0.003,
			Projection::Orthographic(orthographic_projection) => {
				orthographic_projection.near = 0.003
			}
			Projection::Custom(custom_projection) => {
				if let Some(xr) = custom_projection.get_mut::<XrProjection>() {
					xr.near = 0.003
				} else {
					error_once!("unknown custom camera projection");
				}
			}
		}
	}
}

fn add_oit(
	mut commands: Commands,
	cameras: Query<
		Entity,
		(
			With<Camera3d>,
			Without<OrderIndependentTransparencySettings>,
		),
	>,
) {
	for entity in &cameras {
		commands
			.entity(entity)
			.insert(OrderIndependentTransparencySettings {
				layer_count: 4,
				alpha_threshold: 0.00,
			})
			.insert(Msaa::Off);
	}
}

fn xr_step(world: &mut World) {
	// camera::update(token);
	#[cfg(feature = "wayland")]
	Wayland::early_frame(&mut GraphicsInfo {
		_images: world.resource_mut::<Assets<Image>>(),
	});
	destroy_queue::clear();

	// update things like the Xr input methods
	world.run_schedule(PreFrameWait);
	input::process_input();
	let time = world.resource::<bevy::prelude::Time>().delta_secs_f64();
	nodes::root::Root::send_frame_events(time);

	// we are targeting the frame after the wait
	if let Some(mut state) = world.get_resource_mut::<OxrFrameState>() {
		state.predicted_display_time = openxr::Time::from_nanos(
			state.predicted_display_time.as_nanos() + state.predicted_display_period.as_nanos(),
		);
	}

	let should_wait = world
		.run_system_cached(should_run_frame_loop)
		.unwrap_or(false);
	// we might want to do an adaptive sleep when not OpenXR waiting
	if should_wait {
		world.resource_scope::<OxrFrameWaiter, _>(|world, mut waiter| {
			let state = waiter
				.wait()
				.inspect_err(|err| error!("failed to wait OpenXR frame: {err}"))
				.ok();

			if let Some(state) = state {
				world.insert_resource(OxrFrameState(state));
			}
		});
	}

	tick_internal_client();
	#[cfg(feature = "wayland")]
	world.resource_scope::<Assets<BevyMaterial>, _>(|world, mut materials| {
		Wayland::update_graphics(&mut materials, &mut world.resource_mut::<Assets<Image>>());
	});
}

#![recursion_limit = "256"]
#![allow(clippy::empty_docs)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::type_complexity)]
mod bevy_int;
mod core;
mod nodes;
mod objects;
mod session;
#[cfg(feature = "wayland")]
mod wayland;

use bevy::{
	MinimalPlugins,
	a11y::AccessibilityPlugin,
	app::{App, ScheduleRunnerPlugin, TerminalCtrlCHandlerPlugin},
	asset::{AssetMetaCheck, UnapprovedPathMode},
	audio::AudioPlugin,
	core_pipeline::{
		CorePipelinePlugin, oit::OrderIndependentTransparencySettings, tonemapping::Tonemapping,
	},
	diagnostic::DiagnosticsPlugin,
	ecs::schedule::{ExecutorKind, ScheduleLabel},
	gizmos::GizmoPlugin,
	gltf::GltfPlugin,
	input::InputPlugin,
	pbr::PbrPlugin,
	prelude::*,
	render::{
		RenderDebugFlags, RenderPlugin,
		pipelined_rendering::{PipelinedRenderThreadOnCreateCallback, PipelinedRenderingPlugin},
		settings::{Backends, RenderCreation, WgpuSettings},
	},
	scene::ScenePlugin,
	window::{CompositeAlphaMode, PresentMode},
	winit::{WakeUp, WinitPlugin},
};
use bevy_dmabuf::import::DmabufImportPlugin;
use bevy_int::{
	entity_handle::EntityHandlePlugin, spectator_cam::SpectatorCameraPlugin,
	tracking_offset::TrackingOffsetPlugin,
};
use bevy_mod_openxr::{
	action_set_attaching::OxrActionAttachingPlugin,
	action_set_syncing::OxrActionSyncingPlugin,
	add_xr_plugins,
	exts::OxrExtensions,
	features::overlay::OxrOverlaySettings,
	graphics::{GraphicsBackend, OxrManualGraphicsConfig},
	init::{OxrInitPlugin, should_run_frame_loop},
	reference_space::OxrReferenceSpacePlugin,
	render::{OxrRenderPlugin, OxrWaitFrameSystem},
	resources::{OxrFrameState, OxrFrameWaiter, OxrSessionConfig},
	types::AppInfo,
};
use bevy_mod_xr::{
	camera::XrProjection,
	session::{XrFirst, XrHandleEvents, XrSessionPlugin},
};
use clap::Parser;
use core::{
	client::{Client, tick_internal_client},
	task,
};
use directories::ProjectDirs;
use nodes::{
	audio::AudioNodePlugin,
	drawable::{
		lines::LinesNodePlugin, model::ModelNodePlugin, sky::SkyPlugin, text::TextNodePlugin,
	},
	fields::FieldDebugGizmoPlugin,
	input,
	spatial::SpatialNodePlugin,
};
use objects::{
	hmd::HmdPlugin,
	input::{
		mouse_pointer::FlatscreenInputPlugin, oxr_controller::ControllerPlugin,
		oxr_hand::HandPlugin,
	},
	play_space::PlaySpacePlugin,
};
use openxr::{EnvironmentBlendMode, ReferenceSpaceType};
use session::{launch_start, save_session};
use stardust_xr_gluon::object_registry::ObjectRegistry;
use stardust_xr_wire::server::LockedSocket;
use std::{
	ops::DerefMut as _,
	path::PathBuf,
	str::FromStr,
	sync::{Arc, OnceLock},
};
use tokio::{net::UnixListener, sync::Notify, task::JoinError};
use tracing::{error, info, metadata::LevelFilter};
use tracing_subscriber::{EnvFilter, filter::Directive, fmt, prelude::*};
#[cfg(feature = "wayland")]
use wayland::{Wayland, WaylandPlugin};
use zbus::{Connection, fdo::ObjectManager};

#[derive(Debug, Clone, Parser)]
#[clap(author, version, about, long_about = None)]
struct CliArgs {
	/// Force flatscreen mode and use the mouse pointer as a 3D pointer
	#[clap(short, long, action)]
	force_flatscreen: bool,

	/// Replaces the flatscreen mode with a first person spectator camera
	#[clap(short, long, action)]
	spectator: bool,

	/// Creates a transparent window fot the flatscreen mode
	#[clap(short, long, action)]
	transparent_flatscreen: bool,

	/// If monado insists on emulating them, set this flag...we want the raw input
	#[clap(long)]
	disable_controllers: bool,
	/// If monado insists on emulating , set this flag...we want the raw input
	#[clap(long)]
	disable_hands: bool,

	/// Make hands fully transparent for passthrough (useful for wivrn)
	#[clap(long, action)]
	transparent_hands: bool,

	/// Disable pipelined rendering, in case of weird behavior, will decrease performance
	#[clap(long, action)]
	disable_pipelined_rendering: bool,

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
	let registry = registry.with(console_subscriber::spawn());

	let log_layer = fmt::Layer::new()
		.with_thread_names(true)
		.with_ansi(true)
		.with_line_number(true)
		.with_filter(
			EnvFilter::builder()
				.with_default_directive(LevelFilter::WARN.into())
				.from_env_lossy()
				.add_directive(Directive::from_str("bevy_mesh_text_3d::text_glyphs=off").unwrap()),
		);
	registry.with(log_layer).init();

	let cli_args = CliArgs::parse();

	let locked_socket =
		LockedSocket::get_free().expect("Unable to find a free stardust socket path");
	STARDUST_INSTANCE.set(locked_socket.socket_path.file_name().unwrap().to_string_lossy().into_owned()).expect("Someone hasn't done their job, yell at Nova because how is this set multiple times what the hell");
	info!(
		socket_path = ?locked_socket.socket_path.display(),
		"Stardust socket created"
	);
	let socket = UnixListener::bind(locked_socket.socket_path)
		.expect("Couldn't spawn stardust server at {socket_path}");
	task::new(|| "Stardust socket accept loop", async move {
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
	// why is this requested here? should there be a specific server bus name that we check
	// instead?
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

	let object_registry = ObjectRegistry::new(&dbus_connection).await;

	#[cfg(feature = "wayland")]
	let _wayland = Wayland::new().expect("Couldn't create Wayland instance");

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
pub struct ObjectRegistryRes(Arc<ObjectRegistry>);
#[derive(Resource, Deref)]
pub struct DbusConnection(Connection);

fn bevy_loop(
	ready_notifier: Arc<Notify>,
	_project_dirs: Option<ProjectDirs>,
	args: CliArgs,
	dbus_connection: Connection,
	object_registry: Arc<ObjectRegistry>,
) -> AppExit {
	let mut app = App::new();
	app.insert_resource(DbusConnection(dbus_connection));
	app.insert_resource(OxrManualGraphicsConfig {
		fallback_backend: GraphicsBackend::Vulkan(()),
		vk_instance_exts: Vec::new(),
		vk_device_exts: bevy_dmabuf::required_device_extensions(),
	});
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
	#[cfg(feature = "wayland")]
	{
		plugins = plugins.add(DmabufImportPlugin);
	}
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
	if std::env::var("DISPLAY").is_ok_and(|s| !s.is_empty())
		|| std::env::var("WAYLAND_DISPLAY").is_ok_and(|s| !s.is_empty())
	{
		let mut plugin = WinitPlugin::<WakeUp>::default();
		plugin.run_on_any_thread = true;
		plugins = plugins.add(plugin).disable::<ScheduleRunnerPlugin>();
		plugins = match args.spectator {
			true => plugins.add(SpectatorCameraPlugin),
			false => plugins.add(FlatscreenInputPlugin),
		};
	}
	app.insert_resource(PipelinedRenderThreadOnCreateCallback(
		enter_runtime_context.clone(),
	));
	app.add_plugins(
		if !args.force_flatscreen {
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
						exts.khr_convert_timespec_time = true;
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
				// we don't do any action stuff that needs to integrate with the ecosystem
				.disable::<OxrActionAttachingPlugin>()
				.disable::<OxrActionSyncingPlugin>()
		} else {
			// enable a event
			plugins = plugins.add(XrSessionPlugin { auto_handle: false });
			bevy_dmabuf::wgpu_init::add_dmabuf_init_plugin(plugins)
		}
		.set(WindowPlugin {
			primary_window: Some(Window {
				transparent: args.transparent_flatscreen,
				present_mode: PresentMode::AutoNoVsync,
				composite_alpha_mode: if args.transparent_flatscreen {
					CompositeAlphaMode::PreMultiplied
				} else {
					CompositeAlphaMode::Auto
				},
				title: "StardustXR server flatscreen mode".to_string(),
				..default()
			}),
			..default()
		}),
	);
	if !args.disable_pipelined_rendering {
		app.add_plugins(PipelinedRenderingPlugin);
	}

	app.add_plugins(bevy_equirect::EquirectangularPlugin);
	// app.add_plugins(HandGizmosPlugin);
	app.world_mut().resource_mut::<AmbientLight>().brightness = 1000.0;
	if let Some(priority) = args.overlay_priority {
		app.insert_resource(OxrOverlaySettings {
			session_layer_placement: priority,
			..default()
		});
	}
	app.insert_resource(OxrSessionConfig {
		blend_mode_preference: vec![
			EnvironmentBlendMode::ALPHA_BLEND,
			EnvironmentBlendMode::ADDITIVE,
			EnvironmentBlendMode::OPAQUE,
		],
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
		// not really a node ig? at least for now
		SkyPlugin,
	));
	// object plugins
	app.add_plugins((PlaySpacePlugin, HmdPlugin));
	if !args.disable_hands {
		app.add_plugins((
			HandPlugin {
				transparent_hands: args.transparent_hands,
			},
			bevy_sk::hand::HandPlugin,
		));
	}
	if !args.disable_controllers {
		app.add_plugins(ControllerPlugin);
	}

	// feature plugins
	#[cfg(feature = "wayland")]
	app.add_plugins(WaylandPlugin);
	app.add_plugins((TrackingOffsetPlugin, FieldDebugGizmoPlugin));
	app.add_systems(PostStartup, move || {
		ready_notifier.notify_waiters();
	});
	app.add_observer(cam_settings);
	app.add_systems(
		XrFirst,
		xr_step
			.in_set(OxrWaitFrameSystem)
			.in_set(XrHandleEvents::FrameLoop),
	);

	app.run()
}

fn cam_settings(
	trigger: Trigger<OnAdd, Camera3d>,
	mut query: Query<(Entity, &mut Projection, &mut Msaa, &mut Tonemapping), With<Camera3d>>,
	mut cmds: Commands,
) {
	let Ok((entity, mut projection, mut msaa, mut tonemapping)) = query.get_mut(trigger.target())
	else {
		return;
	};
	info!("modifying cam");
	match projection.deref_mut() {
		Projection::Perspective(perspective_projection) => perspective_projection.near = 0.003,
		Projection::Orthographic(orthographic_projection) => orthographic_projection.near = 0.003,
		Projection::Custom(custom_projection) => {
			if let Some(xr) = custom_projection.get_mut::<XrProjection>() {
				xr.near = 0.003
			} else {
				error_once!("unknown custom camera projection");
			}
		}
	}
	*msaa = Msaa::Off;
	*tonemapping = Tonemapping::None;
	cmds.entity(entity)
		.insert(OrderIndependentTransparencySettings::default());
}

fn xr_step(world: &mut World) {
	// update things like the Xr input methods
	world.run_schedule(PreFrameWait);
	input::process_input();
	let time = world.resource::<bevy::prelude::Time>().delta_secs_f64();
	nodes::root::Root::send_frame_events(time);

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
}

pub fn get_time(pipelined: bool, state: &OxrFrameState) -> openxr::Time {
	if pipelined {
		openxr::Time::from_nanos(
			state.predicted_display_time.as_nanos() + state.predicted_display_period.as_nanos(),
		)
	} else {
		state.predicted_display_time
	}
}

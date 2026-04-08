#![recursion_limit = "256"]
#![allow(clippy::empty_docs)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::type_complexity)]
mod bevy_int;
mod core;
mod nodes;
mod session;

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
use bevy_int::{entity_handle::EntityHandlePlugin, spectator_cam::SpectatorCameraPlugin};
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
use directories::ProjectDirs;
use nodes::spatial::SpatialNodePlugin;
// use objects::{
// 	hmd::HmdPlugin,
// 	input::{
// 		mouse_pointer::FlatscreenInputPlugin, oxr_controller::ControllerPlugin,
// 		oxr_hand::HandPlugin,
// 	},
// 	play_space::PlaySpacePlugin,
// };
use openxr::{EnvironmentBlendMode, ReferenceSpaceType};
use pion_binder::PionBinderDevice;
use stardust_xr_protocol::client::FrameInfo;
// use stardust_xr_gluon::object_registry::ObjectRegistry;
// use stardust_xr_wire::server::LockedSocket;
use std::{
	fs,
	ops::DerefMut as _,
	path::PathBuf,
	str::FromStr,
	sync::{Arc, LazyLock, OnceLock},
};
use tokio::{sync::Notify, task::JoinError};
use tracing::{error, info, metadata::LevelFilter};
use tracing_subscriber::{EnvFilter, filter::Directive, fmt, prelude::*};
use zbus::Connection;

use crate::{
	bevy_int::tracking_offset::TrackingOffsetPlugin, core::{client::CLIENTS, server_interface::ServerInterface}, nodes::{
		audio::AudioNodePlugin,
		camera::CameraNodePlugin,
		drawable::{
			lines::LinesNodePlugin, model::ModelNodePlugin, sky::SkyPlugin, text::TextNodePlugin,
		}, fields::FieldDebugGizmoPlugin,
	}, session::{launch_start, save_session}
};

#[derive(Debug, Clone, Parser)]
#[clap(author, version, about, long_about = None)]
struct CliArgs {
	/// Force flatscreen mode and use the mouse pointer as a 3D pointer
	#[clap(short, long, action)]
	force_flatscreen: bool,

	/// Force disable the flatscreen window
	#[clap(short, long, action)]
	xr_only: bool,

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
pub static PION: LazyLock<PionBinderDevice> = LazyLock::new(PionBinderDevice::default);

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

	// let pion =PION;

	let instance = stardust_xr_protocol::dir::find_free_instace()
		.expect("Unable to find a free stardust instance");
	STARDUST_INSTANCE.set(instance.clone()).unwrap();
	let (pion_path, lock) =
		stardust_xr_protocol::dir::create_pion_file("stardust-server", &instance)
			.expect("failed to establish self as server for fresh instance");
	let pion_file = fs::OpenOptions::new()
		.create(true)
		.read(true)
		.write(true)
		.open(&pion_path)
		.expect("failed to open file even tho we're holding a lock file for it");
	info!(
		pion_file_path = ?pion_path.display(),
		"Stardust server pion file created"
	);
	let server_interface = PION.register_object(ServerInterface::default());
	PION.bind_binder_ref_to_file(pion_file, &server_interface)
		.await
		.expect("failed to register server with pion");

	let project_dirs = ProjectDirs::from("", "", "stardust");
	if project_dirs.is_none() {
		error!(
			"Unable to get Stardust project directories, default skybox and startup script will not work."
		);
	}

	let dbus_connection = zbus::conn::Builder::session()
		.unwrap()
		.replace_existing_names(false)
		.allow_name_replacements(false)
		.name(format!("org.stardustxr.Server.{}", instance))
		.expect("Another instance of the server is running with the same STARDUST_INSTANCE")
		.build()
		.await
		.expect("Could not open dbus session");

	let ready_notifier = Arc::new(Notify::new());
	let io_loop = tokio::task::spawn_blocking({
		let ready_notifier = ready_notifier.clone();
		let project_dirs = project_dirs.clone();
		let cli_args = cli_args.clone();
		let dbus_connection = dbus_connection.clone();
		move || bevy_loop(ready_notifier, project_dirs, cli_args, dbus_connection)
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
        // TODO: somehow send SIGTERM instead, we really don't want to send SIGKILL, as that doesn't
        // allow for any cleanup
        // only SIGKILL after a while
		let _ = startup_child.kill();
	}

    let _lock = lock;
	info!("Cleanly shut down Stardust");
	return_value
}

// static DEFAULT_SKYTEX: OnceLock<Tex> = OnceLock::new();
// static DEFAULT_SKYLIGHT: OnceLock<SphericalHarmonics> = OnceLock::new();

#[derive(ScheduleLabel, Hash, Debug, PartialEq, Eq, Clone, Copy)]
pub struct PreFrameWait;
#[derive(Resource, Deref)]
pub struct DbusConnection(Connection);

pub fn vk_device_exts() -> Vec<&'static std::ffi::CStr> {
	let mut exts = bevy_dmabuf::required_device_extensions();
	if !exts.contains(&ash::khr::external_semaphore::NAME) {
		exts.push(ash::khr::external_semaphore::NAME);
	}
	if !exts.contains(&ash::khr::external_semaphore_fd::NAME) {
		exts.push(ash::khr::external_semaphore_fd::NAME);
	}
	exts
}
fn bevy_loop(
	ready_notifier: Arc<Notify>,
	_project_dirs: Option<ProjectDirs>,
	args: CliArgs,
	dbus_connection: Connection,
	// object_registry: Arc<ObjectRegistry>,
) -> AppExit {
	let mut app = App::new();
	app.insert_resource(DbusConnection(dbus_connection));
	app.insert_resource(OxrManualGraphicsConfig {
		fallback_backend: GraphicsBackend::Vulkan(()),
		vk_instance_exts: Vec::new(),
		vk_device_exts: vk_device_exts(),
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
		.add(WindowPlugin::default())
		.add(DmabufImportPlugin);
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
	if (std::env::var("DISPLAY").is_ok_and(|s| !s.is_empty())
		|| std::env::var("WAYLAND_DISPLAY").is_ok_and(|s| !s.is_empty()))
		&& !args.xr_only
	{
		let mut plugin = WinitPlugin::<WakeUp>::default();
		plugin.run_on_any_thread = true;
		plugins = plugins.add(plugin).disable::<ScheduleRunnerPlugin>();
		plugins = match args.spectator {
			true => plugins.add(SpectatorCameraPlugin),
			false => plugins, /* .add(FlatscreenInputPlugin) */
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
						exts.other.push("XR_KHR_generic_controller".to_string());
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
	#[cfg(feature = "bevy_debugging")]
	{
		use bevy::remote::{RemotePlugin, http::RemoteHttpPlugin};
		app.add_plugins((RemotePlugin::default(), RemoteHttpPlugin::default()));
	}
	// the Stardust server plugins
	// infra plugins
	app.add_plugins((
		EntityHandlePlugin,
		// DmatexPlugin,
		// VulkanoPlugin,
	));
	// node plugins
	app.add_plugins((
		SpatialNodePlugin,
		ModelNodePlugin,
		TextNodePlugin,
		LinesNodePlugin,
		AudioNodePlugin,
		CameraNodePlugin,
		// not really a node ig? at least for now
		SkyPlugin,
	));
	// object plugins
	// app.add_plugins((PlaySpacePlugin, HmdPlugin));
	// if !args.disable_hands {
	// 	app.add_plugins((
	// 		HandPlugin {
	// 			transparent_hands: args.transparent_hands,
	// 		},
	// 		bevy_sk::hand::HandPlugin,
	// 	));
	// }
	// if !args.disable_controllers {
	// 	app.add_plugins(ControllerPlugin);
	// }

	// feature plugins
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
	let time = world.resource::<bevy::prelude::Time>().delta_secs_f64();
	for client in CLIENTS.get_vec() {
		_ = client.frame(FrameInfo {
			// TODO: ideally populate with openxr data instead of bevy
			delta: time as f32,
			// TODO: populate with openxr data
			predicted_display_time: None,
		});
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

	// tick_internal_client();
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

#![allow(clippy::empty_docs)]
pub mod bevy_plugin;
mod core;
mod nodes;
mod objects;
pub mod oxr_render_plugin;
mod session;
#[cfg(feature = "wayland")]
mod wayland;

use crate::core::destroy_queue;
// use crate::nodes::items::camera;
use crate::nodes::{audio, drawable, input};

use bevy::a11y::AccessibilityPlugin;
use bevy::app::{
	App, AppExit, PluginGroup, PluginsState, PostUpdate, ScheduleRunnerPlugin, Startup,
	TerminalCtrlCHandlerPlugin, Update,
};
use bevy::asset::{AssetPlugin, AssetServer, Handle};
use bevy::audio::AudioPlugin;
use bevy::color::Color;
use bevy::core_pipeline::{CorePipelinePlugin, Skybox};
use bevy::gizmos::GizmoPlugin;
use bevy::gltf::GltfPlugin;
use bevy::image::Image;
use bevy::log::LogPlugin;
use bevy::pbr::{PbrPlugin, StandardMaterial};
use bevy::prelude::{
	on_event, resource_added, Camera3d, ClearColor, Commands, Entity, EventReader, HierarchyPlugin,
	ImagePlugin, IntoSystemConfigs, Local, Query, Res, ResMut, Resource, Transform,
	TransformPlugin, With, World,
};
use bevy::render::pipelined_rendering::PipelinedRenderingPlugin;
use bevy::render::RenderPlugin;
use bevy::scene::ScenePlugin;
use bevy::time::Time;
use bevy::utils::default;
use bevy::window::WindowPlugin;
use bevy::winit::{EventLoopProxyWrapper, WakeUp, WinitPlugin};
use bevy::{DefaultPlugins, MinimalPlugins};
use bevy_mod_openxr::action_set_syncing::{OxrActionSyncingPlugin, OxrSyncActionSet};
use bevy_mod_openxr::exts::OxrExtensions;
use bevy_mod_openxr::features::overlay::{OxrOverlaySessionEvent, OxrOverlaySettings};
use bevy_mod_openxr::init::{should_run_frame_loop, OxrInitPlugin};
use bevy_mod_openxr::render::{update_cameras, OxrRenderPlugin};
use bevy_mod_openxr::resources::{OxrFrameState, OxrFrameWaiter, OxrGraphicsInfo};
use bevy_mod_openxr::session::OxrSession;
use bevy_mod_openxr::spaces::OxrSpaceExt;
use bevy_mod_openxr::types::{AppInfo, Version};
use bevy_mod_openxr::{add_xr_plugins, openxr_session_running};
use bevy_mod_xr::session::{XrFirst, XrPreDestroySession, XrSessionCreated, XrSessionPlugin};
use bevy_mod_xr::spaces::XrPrimaryReferenceSpace;
use bevy_plugin::{DbusConnection, InputUpdate, StardustBevyPlugin, StardustFirst};
use clap::Parser;
use color_eyre::eyre::eyre;
use core::client::Client;
use core::task;
use directories::ProjectDirs;
use nodes::audio::StardustSoundPlugin;
use nodes::drawable::lines::BevyLinesPlugin;
use nodes::drawable::model::StardustModelPlugin;
use nodes::drawable::text::StardustTextPlugin;
use objects::input::sk_controller::StardustControllerPlugin;
use objects::input::sk_hand::StardustHandPlugin;
use objects::ServerObjects;
use once_cell::sync::OnceCell;
use openxr::OverlaySessionCreateFlagsEXTX;
use oxr_render_plugin::StardustOxrRenderPlugin;
use session::{launch_start, save_session};
use stardust_xr::schemas::dbus::object_registry::ObjectRegistry;
use stardust_xr::server;
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use tokio::net::UnixListener;
use tokio::sync::Notify;
use tracing::level_filters::LevelFilter;
use tracing::{debug_span, error, info, warn};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use zbus::fdo::ObjectManager;
use zbus::Connection;

pub type DefaultMaterial = StandardMaterial;

#[derive(Debug, Clone, Parser)]
#[clap(author, version, about, long_about = None)]
struct CliArgs {
	/// Force flatscreen mode and use the mouse pointer as a 3D pointer
	#[clap(short, long, action)]
	flatscreen: bool,

	/// Force Pipelined Rending, improving fps at the cost of latency
	#[clap(short, long, action)]
	pipelined_rendering: bool,

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

static STARDUST_INSTANCE: OnceCell<String> = OnceCell::new();
static TOKIO: RuntimeWrapper = RuntimeWrapper(OnceCell::new());

struct RuntimeWrapper(OnceCell<tokio::runtime::Runtime>);
impl Deref for RuntimeWrapper {
	type Target = tokio::runtime::Runtime;

	fn deref(&self) -> &Self::Target {
		self.0.get().unwrap()
	}
}

fn main() -> color_eyre::Result<AppExit> {
	let runtime = tokio::runtime::Builder::new_multi_thread()
		.enable_all()
		.build()?;
	TOKIO.0.set(runtime).unwrap();
	TOKIO.block_on(setup())
}
async fn setup() -> color_eyre::Result<AppExit> {
	color_eyre::install()?;

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

	let dbus_connection = Connection::session()
		.await
		.expect("Could not open dbus session");
	dbus_connection
		.request_name("org.stardustxr.HMD")
		.await
		.expect("Another instance of the server is running. This is not supported currently (but is planned).");

	dbus_connection
		.object_server()
		.at("/", ObjectManager)
		.await
		.expect("Couldn't add the object manager");

	let object_registry = ObjectRegistry::new(&dbus_connection).await.expect(
		"Couldn't make the object registry to find all objects with given interfaces in d-bus",
	);

	let sk_ready_notifier = Arc::new(Notify::new());
	let stereokit_loop = tokio::task::spawn_blocking({
		let sk_ready_notifier = sk_ready_notifier.clone();
		let project_dirs = project_dirs.clone();
		let cli_args = cli_args.clone();
		let dbus_connection = dbus_connection.clone();
		move || {
			bevy_loop(
				sk_ready_notifier,
				project_dirs,
				cli_args,
				dbus_connection,
				object_registry,
				false,
			)
		}
	});
	sk_ready_notifier.notified().await;
	let mut startup_children = project_dirs
		.as_ref()
		.map(|project_dirs| launch_start(&cli_args, project_dirs))
		.unwrap_or_default();

	let exit = stereokit_loop.await?;
	info!("Stopping...");
	if let Some(project_dirs) = project_dirs {
		save_session(&project_dirs).await;
	}
	for mut startup_child in startup_children.drain(..) {
		let _ = startup_child.kill();
	}

	info!("Cleanly shut down Stardust");
	Ok(exit)
}

fn bevy_loop(
	sk_ready_notifier: Arc<Notify>,
	project_dirs: Option<ProjectDirs>,
	args: CliArgs,
	dbus_connection: Connection,
	object_registry: ObjectRegistry,
	headless: bool,
) -> AppExit {
	let mut bevy_app = App::new();
	// let base = (DefaultPlugins)
	// 	.build()
	// 	.disable::<PipelinedRenderingPlugin>()
	// 	.disable::<LogPlugin>()
	// 	.set({
	// 		let mut plugin = WinitPlugin::<WakeUp>::default();
	// 		plugin.run_on_any_thread = true;
	// 		plugin
	// 	});
	let mut base = (MinimalPlugins)
		.build()
		.disable::<ScheduleRunnerPlugin>()
		.add(TransformPlugin)
		.add(HierarchyPlugin)
		.add(AccessibilityPlugin);
	base = match headless {
		true => {
			base.add(ScheduleRunnerPlugin {
				// In OpenXR framepacing we trust (else this will eat all of the cpu)
				run_mode: bevy::app::RunMode::Loop { wait: None },
			})
		}
		false => base.add(WindowPlugin::default()).add({
			let mut plugin = WinitPlugin::<WakeUp>::default();
			plugin.run_on_any_thread = true;
			plugin
		}),
	};
	base = base
		.add(TerminalCtrlCHandlerPlugin)
		// might want to modify this in the future?
		.add(AssetPlugin::default())
		// will be replaced by bevy_mod_openxr when using OpenXR
		.add(RenderPlugin::default())
		.add(ImagePlugin::default())
		.add(CorePipelinePlugin)
		// very unsure what is needed here
		.add(PbrPlugin {
			// hoping that there is very little overdraw in stardust
			prepass_enabled: false,
			add_default_deferred_lighting_plugin: true,
			use_gpu_instance_buffer_builder: true,
		})
		.add(ScenePlugin)
		.add(GltfPlugin::default())
		.add(AudioPlugin::default())
		.add(GizmoPlugin);

	if args.pipelined_rendering {
		base = base.add(PipelinedRenderingPlugin);
	}

	if args.flatscreen {
		bevy_app.add_plugins(base);
	} else {
		bevy_app.add_plugins(
			add_xr_plugins(base)
				.set(OxrInitPlugin {
					app_info: AppInfo {
						name: "Stardust XR".into(),
						version: Version(0, 44, 1),
					},
					exts: {
						let mut exts = OxrExtensions::default();
						exts.enable_hand_tracking();
						if args.overlay_priority.is_some() {
							exts.enable_extx_overlay();
						}
						exts
					},
					blend_modes: Some(vec![
						openxr::EnvironmentBlendMode::ALPHA_BLEND,
						openxr::EnvironmentBlendMode::OPAQUE,
					]),
					synchronous_pipeline_compilation: false,
					..Default::default()
				})
				.disable::<OxrRenderPlugin>()
				.disable::<OxrActionSyncingPlugin>()
				.add_after::<OxrInitPlugin>(StardustOxrRenderPlugin),
		);
		if let Some(priority) = args.overlay_priority {
			bevy_app.insert_resource(OxrOverlaySettings {
				session_layer_placement: priority,
				flags: OverlaySessionCreateFlagsEXTX::EMPTY,
			});
		}
		bevy_app.add_event::<OxrSyncActionSet>();
		bevy_app.add_plugins(bevy_xr_utils::hand_gizmos::HandGizmosPlugin);
	}
	bevy_app.add_plugins(StardustBevyPlugin);
	bevy_app.add_plugins((
		BevyLinesPlugin,
		StardustModelPlugin,
		StardustHandPlugin,
		// StardustTextPlugin,
		StardustSoundPlugin,
		StardustControllerPlugin,
	));
	#[derive(Resource)]
	struct SkyTexture(Handle<Image>);
	// Skytex/light stuff
	bevy_app.add_systems(
		Startup,
		move |assests: Res<AssetServer>, mut cmds: Commands| {
			if let Some(sky) = project_dirs
				.as_ref()
				.map(|dirs| dirs.config_dir().join("skytex.hdr"))
				.filter(|f| f.exists())
				.map(|p| assests.load(p))
			{
				cmds.insert_resource(SkyTexture(sky));
			}
		},
	);
	#[derive(Resource)]
	struct RenderBackground(bool);
	fn update_background(
		graphics_info: Res<OxrGraphicsInfo>,
		mut overlay_events: EventReader<OxrOverlaySessionEvent>,
		mut last_hidden: Local<bool>,
		cams: Query<Entity, With<Camera3d>>,
		mut cmds: Commands,
	) {
		let env_hidden = graphics_info.blend_mode != openxr::EnvironmentBlendMode::OPAQUE
			&& overlay_events
				.read()
				.last()
				.map(
					|OxrOverlaySessionEvent::MainSessionVisibilityChanged { visible, flags: _ }| {
						*visible
					},
				)
				.unwrap_or(true);
		if env_hidden && !*last_hidden {
			cams.iter().for_each(|e| {
				cmds.entity(e).remove::<Skybox>();
			});
			let _span = debug_span!("spawn");
			cmds.insert_resource(ClearColor(Color::NONE));
		}
		*last_hidden = env_hidden;
	}
	bevy_app.add_systems(XrSessionCreated, update_background);
	bevy_app.add_systems(
		PostUpdate,
		(|mut objects: ResMut<ServerObjects>,
		  ref_space: Res<XrPrimaryReferenceSpace>,
		  session: Res<OxrSession>| {
			objects
				.ref_space
				.replace(unsafe { ref_space.as_openxr_space(&session) });
			objects.view_space.replace(
				session
					.deref()
					.deref()
					.create_reference_space(
						openxr::ReferenceSpaceType::VIEW,
						openxr::Posef::IDENTITY,
					)
					.unwrap(),
			);
		})
		.run_if(on_event::<bevy_mod_xr::session::XrSessionCreatedEvent>),
	);
	bevy_app.add_systems(XrPreDestroySession, |mut objetcs: ResMut<ServerObjects>| {
		objetcs.ref_space = None;
		objetcs.view_space = None;
	});
	bevy_app.add_systems(
		Update,
		update_background.run_if(on_event::<OxrOverlaySessionEvent>),
	);
	bevy_app.insert_resource(DbusConnection(dbus_connection.clone()));

	#[cfg(feature = "wayland")]
	let mut wayland = wayland::Wayland::new().expect("Could not initialize wayland");
	#[cfg(feature = "wayland")]
	wayland.make_context_current();
	sk_ready_notifier.notify_waiters();
	info!("Stardust ready!");

	let mut objects = ServerObjects::new(dbus_connection.clone());
	fn sync_sets(session: Res<OxrSession>, mut events: EventReader<OxrSyncActionSet>) {
		let sets = events
			.read()
			.map(|v| &v.0)
			.map(openxr::ActiveActionSet::new)
			.collect::<Vec<_>>();
		if sets.is_empty() {
			return;
		}

		if let Err(err) = session.sync_actions(&sets) {
			warn!("error while syncing actionsets: {}", err.to_string());
		}
	}

	bevy_app.insert_resource(objects);

	fn bevy_step(world: &mut World) {
		let _span = debug_span!("Bevy step");
		let _span = _span.enter();
		// camera::update(token);
		#[cfg(feature = "wayland")]
		wayland.frame_event();
		destroy_queue::clear();

		let time = world.get_resource_mut::<OxrFrameState>().map(|mut s| {
			let t = openxr::Time::from_nanos(
				s.predicted_display_time.as_nanos() + s.predicted_display_period.as_nanos(),
			);
			s.predicted_display_time = t;
			t
		});
		world.run_schedule(XrFirst);
		if world
			.run_system_cached(openxr_session_running)
			.unwrap_or(true)
		{
			world.run_system_cached(sync_sets);
		}
		let thread = world
			.run_system_cached(should_run_frame_loop)
			.unwrap_or(true)
			.then(|| world.remove_resource::<OxrFrameWaiter>())
			.flatten()
			.map(|mut waiter| {
				TOKIO.spawn_blocking(move || {
					let _span = debug_span!("eeping").entered();
					let result = waiter.wait();
					(waiter, result)
				})
			});
		world.run_schedule(InputUpdate);
		debug_span!("update_objects").in_scope(|| {
			let session = world.remove_resource::<OxrSession>();
			let mut objects = world.remove_resource::<ServerObjects>().unwrap();
			objects.update(session.as_deref(), time);
			world.insert_resource(objects);
			if let Some(session) = session {
				world.insert_resource(session);
			}
		});
		input::process_input();
		nodes::root::Root::send_frame_events(world.resource::<Time>().delta_secs_f64());
		if let Some((waiter, Ok(state))) = thread.map(|t| TOKIO.block_on(t).unwrap()) {
			world.insert_resource(OxrFrameState(state));
			world.insert_resource(waiter);
			if let Err(err) = world.run_system_cached(update_cameras) {
				error!("error while running oxr update_cameras system: {err}");
			}
		}
		#[cfg(feature = "wayland")]
		wayland.update();
	};
	bevy_app.add_systems(StardustFirst, bevy_step);
	let out = bevy_app.run();

	#[cfg(feature = "wayland")]
	drop(wayland);
	out
}

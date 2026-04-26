#![allow(dead_code)]
use crate::PION;
use crate::core::vulkano_data::VULKANO_CONTEXT;
use crate::exposed_interface;
use crate::nodes::ProxyExt;
use crate::nodes::drawable::dmatex::Dmatex;
use crate::nodes::drawable::dmatex::DmatexExt as _;
use crate::nodes::drawable::dmatex::SignalOnDrop;
use crate::nodes::spatial::SpatialObject;
use crate::nodes::spatial::TransformExt as _;
use bevy::app::App;
use bevy::app::Plugin;
use bevy::app::Update;
use bevy::core_pipeline::core_3d::Camera3d;
use bevy::ecs::component::Component;
use bevy::ecs::entity::Entity;
use bevy::ecs::name::Name;
use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::ecs::system::Commands;
use bevy::ecs::system::Query;
use bevy::render::Render;
use bevy::render::RenderApp;
use bevy::render::RenderSet;
use bevy::render::camera::Projection;
use bevy::render::camera::RenderTarget;
use bevy::render::extract_component::ExtractComponent;
use bevy::render::extract_component::ExtractComponentPlugin;
use bevy_mod_xr::camera::XrProjection;
use binderbinder::binder_object::BinderObjectRef;
use glam::Mat4;
use gluon_wire::impl_transaction_handler;
use parking_lot::Mutex;
use stardust_xr_protocol::camera::Camera as CameraProxy;
use stardust_xr_protocol::camera::CameraHandler;
use stardust_xr_protocol::camera::CameraInterfaceHandler;
use stardust_xr_protocol::camera::View;
use stardust_xr_protocol::dmatex::DmatexRef;
use stardust_xr_server_foundation::registry::Registry;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::error;
use tracing::warn;
use vulkano::VulkanObject;
use vulkano::sync::semaphore::ExternalSemaphoreHandleTypes;
use vulkano::sync::semaphore::Semaphore;
use vulkano::sync::semaphore::SemaphoreCreateInfo;
use wgpu_hal::vulkan::SIGNAL_SEMAPHORES;
use wgpu_hal::vulkan::WAIT_SEMAPHORES;

#[derive(Debug)]
pub struct Camera {
	spatial: Arc<SpatialObject>,
	queued_render_targets:
		Mutex<mpsc::UnboundedReceiver<(u64, Vec<View>, Arc<Dmatex>, SignalOnDrop)>>,
	render_target_queue: mpsc::UnboundedSender<(u64, Vec<View>, Arc<Dmatex>, SignalOnDrop)>,
}
impl Camera {
	pub fn new(spatial: Arc<SpatialObject>) -> BinderObjectRef<Camera> {
		let (tx, rx) = mpsc::unbounded_channel();
		let cam = PION.register_object(Camera {
			spatial,
			queued_render_targets: Mutex::new(rx),
			render_target_queue: tx,
		});
		CAMERA_REGISTRY.add_raw(cam.handler_arc());
		cam.to_service()
	}
}
impl CameraHandler for Camera {
	async fn request_draw(
		&self,
		_ctx: gluon_wire::GluonCtx,
		render_target: DmatexRef,
		acquire_point: u64,
		release_point: u64,
		views: Vec<View>,
	) {
		let Some(tex) = render_target.owned() else {
			error!("tried to render to an unknown dmatex");
			return;
		};
		let tx = self.render_target_queue.clone();
		let release_on_drop = tex.signal_on_drop(release_point);
		tokio::spawn(async move {
			let Ok(future) = tex
				.timeline_sync()
				.wait_async(acquire_point)
				.inspect_err(|err| error!("unable to async wait on dmatex timeline: {err}"))
			else {
				return;
			};
			future.await;
			tx.send((acquire_point, views, tex.handler_arc().clone(), release_on_drop))
				.unwrap();
		});
	}
}
static CAMERA_REGISTRY: Registry<Camera> = Registry::new();

// TODO: figure out where to mount this
exposed_interface!(CameraInterface, "stardust-camera");
impl CameraInterfaceHandler for CameraInterface {
	async fn create_camera(
		&self,
		_ctx: gluon_wire::GluonCtx,
		spatial: stardust_xr_protocol::spatial::Spatial,
	) -> CameraProxy {
		let Some(spatial) = spatial.owned() else {
			// TODO: just return an error
			panic!("Invalid Spatial use to create camera");
		};
		let cam = Camera::new(spatial.handler_arc().clone());
		CameraProxy::from_handler(&cam)
	}
}
impl_transaction_handler!(Camera);
pub struct CameraNodePlugin;
impl Plugin for CameraNodePlugin {
	fn build(&self, app: &mut App) {
		app.add_plugins(ExtractComponentPlugin::<CameraReleaseSignal>::default());
		app.add_systems(Update, update_cameras);
		app.sub_app_mut(RenderApp)
			.add_systems(Render, setup_release_semaphore.in_set(RenderSet::Prepare))
			.add_systems(Render, release_end_of_render.in_set(RenderSet::Cleanup));
	}
}

fn setup_release_semaphore(query: Query<(Entity, &CameraReleaseSignal)>, mut cmds: Commands) {
	let vk = VULKANO_CONTEXT.wait();
	for (entity, sync) in &query {
		let lock = sync.0.lock();
		let Some(v) = lock.as_ref() else {
			continue;
		};
		v.1.tex_id();
		let sema = Semaphore::new(
			vk.dev.clone(),
			SemaphoreCreateInfo {
				export_handle_types: ExternalSemaphoreHandleTypes::SYNC_FD,
				..Default::default()
			},
		)
		.unwrap();
		let raw_sema = sema.handle();
		SIGNAL_SEMAPHORES.lock().push(raw_sema);
		cmds.entity(entity).insert(CameraReleaseSemaphore(sema));
	}
}

fn release_end_of_render(mut query: Query<(&mut CameraReleaseSignal, &CameraReleaseSemaphore)>) {
	for (mut signal, sema) in &mut query {
		if let Some((_, signal_on_drop)) = signal.0.get_mut().take() {
			signal_on_drop.use_semaphore(&sema.0);
		};
	}
}

#[derive(Component)]
struct CameraReleaseSignal(Mutex<Option<(Semaphore, SignalOnDrop)>>);
#[derive(Component)]
struct CameraReleaseSemaphore(Semaphore);

impl ExtractComponent for CameraReleaseSignal {
	type QueryData = &'static Self;

	type QueryFilter = ();

	type Out = Self;

	fn extract_component(
		item: bevy::ecs::query::QueryItem<'_, Self::QueryData>,
	) -> Option<Self::Out> {
		Some(Self(
			item.0
				.lock()
				.take()
				.inspect(|(sema, _)| {
					WAIT_SEMAPHORES.lock().push(sema.handle());
				})
				.into(),
		))
	}
}

use bevy::render::camera::Camera as BevyCamera;
fn update_cameras(mut query: Query<(&mut BevyCamera, &mut Projection)>, mut cmds: Commands) {
	for cam in CAMERA_REGISTRY.get_valid_contents() {
		let Some(entity) = cam.spatial.get_entity() else {
			continue;
		};
		let Ok((mut camera, mut projection)) = query.get_mut(entity) else {
			_ = cmds.get_entity(entity).map(|mut c| {
				c.insert((
					Name::new("CameraNode"),
					Camera3d::default(),
					BevyCamera {
						// clear_color: ClearColorConfig::Custom(Color::WHITE),
						is_active: false,
						..Default::default()
					},
					Projection::custom(XrProjection::default()),
				));
			});
			continue;
		};
		// camera.is_active = false;

		let Some((acquire_point, views, tex, signal_on_drop)) =
			cam.queued_render_targets.lock().try_recv().ok()
		else {
			continue;
		};

		let sema = tex.get_acquire_semaphore(acquire_point);
		let Some(view) = views.first() else {
			warn!("render task submitted without view");
			continue;
		};
		let offset_mat = view.camera_relative_transform.to_mat4();

		let Projection::Custom(proj) = projection.as_mut() else {
			warn!("incorrect proj");
			continue;
		};
		let Some(proj) = proj.get_mut::<XrProjection>() else {
			warn!("incorrect custom proj");
			continue;
		};
		proj.projection_matrix = view.projection_matrix.mint::<Mat4>() * (offset_mat.inverse());

		let Some(view_handle) = tex.try_get_bevy_manual_view() else {
			continue;
		};
		camera.target = RenderTarget::TextureView(view_handle);
		camera.is_active = true;
		cmds.entity(entity)
			.insert(CameraReleaseSignal(Some((sema, signal_on_drop)).into()));
	}
}

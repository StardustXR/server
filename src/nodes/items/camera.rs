#![allow(dead_code)]
use crate::core::client::Client;
use crate::core::error::Result;
use crate::core::vulkano_data::VULKANO_CONTEXT;
use crate::nodes::Aspect;
use crate::nodes::AspectIdentifier;
use crate::nodes::Id;
use crate::nodes::Node;
use crate::nodes::drawable::DmatexSubmitInfo;
use crate::nodes::drawable::dmatex::ImportedDmatex;
use crate::nodes::drawable::dmatex::SignalOnDrop;
use crate::nodes::spatial::SPATIAL_ASPECT_ALIAS_INFO;
use crate::nodes::spatial::{Spatial, Transform};
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
use glam::Mat4;
use parking_lot::Mutex;
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

stardust_xr_server_codegen::codegen_camera_protocol!();

pub struct Camera {
	spatial: Arc<Spatial>,
	queued_render_targets:
		Mutex<mpsc::UnboundedReceiver<(u64, Vec<View>, Arc<ImportedDmatex>, SignalOnDrop)>>,
	render_target_queue: mpsc::UnboundedSender<(u64, Vec<View>, Arc<ImportedDmatex>, SignalOnDrop)>,
}
#[allow(unused)]
impl Camera {
	pub fn add_to(node: &Arc<Node>) -> Arc<Camera> {
		let (tx, rx) = mpsc::unbounded_channel();
		let cam = node.add_aspect(Camera {
			spatial: node.get_aspect::<Spatial>().unwrap().clone(),
			queued_render_targets: Mutex::new(rx),
			render_target_queue: tx,
		});
		CAMERA_REGISTRY.add_raw(&cam);
		cam
	}
}
impl AspectIdentifier for Camera {
	impl_aspect_for_camera_aspect_id! {}
}
impl Aspect for Camera {
	impl_aspect_for_camera_aspect! {}
}
impl CameraAspect for Camera {
	fn request_draw(
		node: Arc<Node>,
		calling_client: Arc<Client>,
		render_target: DmatexSubmitInfo,
		views: Vec<View>,
	) -> Result<()> {
		let cam = node.get_aspect::<Camera>()?;
		let Some(tex) = calling_client.dmatexes.get(&render_target.dmatex_id) else {
			error!("invalid dmatex id: {}", render_target.dmatex_id);
			return Ok(());
		};
		let tex = tex.clone();
		let tx = cam.render_target_queue.clone();
		let release_on_drop = tex.signal_on_drop(render_target.release_point.0);
		tokio::spawn(async move {
			let Ok(future) = tex
				.timeline_sync()
				.wait_async(render_target.acquire_point.0)
				.inspect_err(|err| error!("unable to async wait on dmatex timeline: {err}"))
			else {
				return;
			};
			future.await;
			tx.send((render_target.acquire_point.0, views, tex, release_on_drop))
				.unwrap();
		});
		Ok(())
	}
}
impl Drop for Camera {
	fn drop(&mut self) {
		CAMERA_REGISTRY.remove(self);
	}
}
static CAMERA_REGISTRY: Registry<Camera> = Registry::new();

impl InterfaceAspect for Interface {
	fn create_camera(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		id: Id,
		parent: Arc<Node>,
		transform: Transform,
	) -> Result<()> {
		let node = Node::from_id(&calling_client, id, true);
		let parent = parent.get_aspect::<Spatial>()?;
		let transform = transform.to_mat4(true, true, true);
		let node = node.add_to_scenegraph()?;
		Spatial::add_to(&node, Some(parent.clone()), transform);
		Camera::add_to(&node);
		Ok(())
	}
}
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
		let offset_mat = view.camera_relative_transform.to_mat4(true, true, true);

		let Projection::Custom(proj) = projection.as_mut() else {
			warn!("incorrect proj");
			continue;
		};
		let Some(proj) = proj.get_mut::<XrProjection>() else {
			warn!("incorrect custom proj");
			continue;
		};
		proj.projection_matrix =
			Mat4::from(view.projection_matrix) * (Mat4::from(offset_mat).inverse());

		let Some(view_handle) = tex.try_get_bevy_manual_view() else {
			continue;
		};
		camera.target = RenderTarget::TextureView(view_handle);
		camera.is_active = true;
		cmds.entity(entity)
			.insert(CameraReleaseSignal(Some((sema, signal_on_drop)).into()));
	}
}

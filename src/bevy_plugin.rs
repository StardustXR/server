use std::ops::Deref;

use bevy::{
	app::MainScheduleOrder,
	ecs::schedule::{ExecutorKind, ScheduleLabel},
	math::bounding::Aabb3d,
	pbr::{DefaultOpaqueRendererMethod, GpuMeshPreprocessPlugin, MeshRenderPlugin},
	prelude::*,
	render::extract_resource::ExtractResourcePlugin,
};
use bevy_mod_openxr::session::OxrSession;
use bevy_mod_xr::session::{session_available, XrFirst, XrSessionCreated};
use once_cell::sync::OnceCell;
use openxr::ReferenceSpaceType;
use stardust_xr::values::color::{color_space::LinearRgb, AlphaColor, Rgb};

use crate::DefaultMaterial;

pub struct StardustBevyPlugin;

pub static DESTROY_ENTITY: DestroySender = DestroySender(OnceCell::new());

pub struct DestroySender(OnceCell<crossbeam_channel::Sender<Entity>>);
impl Deref for DestroySender {
	type Target = crossbeam_channel::Sender<Entity>;

	fn deref(&self) -> &Self::Target {
		self.0.get().unwrap()
	}
}
#[derive(Resource, Deref)]
struct DestroyEntityReader(crossbeam_channel::Receiver<Entity>);

#[derive(Resource, Deref)]
pub struct DbusConnection(pub zbus::Connection);

#[derive(ScheduleLabel, Hash, Debug, PartialEq, Eq, Clone)]
pub struct InputUpdate;
#[derive(ScheduleLabel, Hash, Debug, PartialEq, Eq, Clone)]
pub struct StardustFirst;
impl Plugin for StardustBevyPlugin {
	fn build(&self, app: &mut App) {
		let (tx, rx) = crossbeam_channel::unbounded();
		DESTROY_ENTITY
			.0
			.set(tx)
			.expect("unable to set destroy entity sender, yell at schmarni pls thx");
		app.insert_resource(DestroyEntityReader(rx));
		app.init_schedule(StardustExtract);
		let labels = &mut app.world_mut().resource_mut::<MainScheduleOrder>().labels;
		info!("test: {labels:?}");
		labels.insert(labels.len() - 2, StardustExtract.intern());
		app.add_systems(Startup, spawn_camera.run_if(not(session_available)));
		app.add_systems(XrSessionCreated, make_view_space);
		let mut schedule = Schedule::new(InputUpdate);
		schedule.set_executor_kind(ExecutorKind::Simple);
		app.add_schedule(schedule);

		let mut schedule = Schedule::new(StardustFirst);
		schedule.set_executor_kind(ExecutorKind::Simple);
		app.add_schedule(schedule);

		let labels = &mut app.world_mut().resource_mut::<MainScheduleOrder>().labels;
		let xr_first_intern = (XrFirst).intern();
		if labels.remove(0) != xr_first_intern {
			panic!("first schedule was not XrFirst!");
		}
		labels.insert(0, (StardustFirst).intern());
		app.add_systems(First, yeet_entities);
		// app.add_observer(
		// 	|trigger: Trigger<OnInsert, MeshMaterial3d<DefaultMaterial>>,
		// 	 query: Query<&MeshMaterial3d<DefaultMaterial>>,
		// 	 mut mats: ResMut<Assets<DefaultMaterial>>| {
		// 		let Ok(handle) = query.get(trigger.entity()) else {
		// 			return;
		// 		};
		// 		if let Some(mat) = mats.get_mut(handle) {
		// 			if matches!(mat.alpha_mode, AlphaMode::Blend | AlphaMode::Mask(_)) {
		// 				mat.alpha_mode = AlphaMode::AlphaToCoverage
		// 			}
		// 		}
		// 	},
		// );
	}
}

fn yeet_entities(
	mut cmds: Commands,
	query: Query<Entity, With<TemporaryEntity>>,
	reader: Res<DestroyEntityReader>,
) {
	query.iter().for_each(|e| {
		info!("yeeting component entities");
		cmds.entity(e).despawn_recursive();
	});
	reader
		.0
		.try_iter()
		.for_each(|e| cmds.entity(e).despawn_recursive());
}

fn make_view_space(mut cmds: Commands, session: Res<OxrSession>) {
	// idk what errors this function returns
	let view_space = session
		.create_reference_space(ReferenceSpaceType::VIEW, Transform::IDENTITY)
		.unwrap();
	// this locates the view space against the default reference space (stage i belive) and sets
	// the transform relative to the XrTrackingRoot
	cmds.spawn((view_space.0, ViewLocation));
}

fn spawn_camera(mut cmds: Commands) {
	cmds.spawn((Camera3d::default(), ViewLocation));
}
pub trait StardustAabb3dExt {
	fn grown_box(&self, aabb: &Self, opt_box_transform: Option<impl Into<Mat4>>) -> Self;
	fn grown_point(&self, pt: impl Into<Vec3>) -> Self;
}
impl StardustAabb3dExt for Aabb3d {
	fn grown_box(&self, other: &Self, opt_box_transform: Option<impl Into<Mat4>>) -> Self {
		let mat = opt_box_transform.map(|m| m.into());
		let other_min = mat
			.as_ref()
			.map(|v| v.transform_point3a(other.min))
			.unwrap_or(other.min);
		let other_max = mat
			.as_ref()
			.map(|v| v.transform_point3a(other.max))
			.unwrap_or(other.max);
		let tmp = self.grown_point(other_min);
		tmp.grown_point(other_max)
	}

	fn grown_point(&self, pt: impl Into<Vec3>) -> Self {
		let pt = pt.into();
		let mut min = self.min;
		let mut max = self.max;
		if pt.x > max.x {
			max.x = pt.x;
		} else if pt.x < min.x {
			min.x = pt.x;
		}
		if pt.y > max.y {
			max.y = pt.y;
		} else if pt.y < min.y {
			min.y = pt.y;
		}
		if pt.z > max.z {
			max.z = pt.z;
		} else if pt.z < min.z {
			min.z = pt.z;
		}

		Aabb3d { min, max }
	}
}

pub const fn convert_linear_rgba(c: AlphaColor<f32, Rgb<f32, LinearRgb>>) -> LinearRgba {
	LinearRgba {
		red: c.c.r,
		green: c.c.g,
		blue: c.c.b,
		alpha: c.a,
	}
}

#[derive(ScheduleLabel, Hash, Debug, Clone, Copy, PartialEq, Eq)]
pub struct StardustExtract;
#[derive(Component, Hash, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TemporaryEntity;
#[derive(Component, Hash, Debug, Clone, Copy, PartialEq, Eq)]
#[require(GlobalTransform)]
pub struct ViewLocation;
#[derive(Hash, Debug, Clone, Copy, PartialEq, Eq, Deref)]
pub struct MainWorldEntity(pub Entity);

pub struct DummyPbrPlugin;

impl Plugin for DummyPbrPlugin {
	fn build(&self, app: &mut App) {
		let use_gpu_instance_buffer_builder = true;
		app.init_asset::<StandardMaterial>();
		app.add_plugins((
			GpuMeshPreprocessPlugin {
				use_gpu_instance_buffer_builder,
			},
			MeshRenderPlugin {
				use_gpu_instance_buffer_builder,
			},
		));
		app.register_type::<DefaultOpaqueRendererMethod>()
			.init_resource::<DefaultOpaqueRendererMethod>()
			.add_plugins(ExtractResourcePlugin::<DefaultOpaqueRendererMethod>::default());
	}
}

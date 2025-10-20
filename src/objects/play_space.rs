use std::sync::Arc;

use bevy::prelude::*;
use bevy_mod_openxr::{
	helper_traits::{ToQuat, ToVec3},
	resources::{OxrFrameState, Pipelined},
	session::OxrSession,
};
use bevy_mod_xr::{
	session::{XrPreDestroySession, XrSessionCreated},
	spaces::{XrPrimaryReferenceSpace, XrReferenceSpace, XrSpace},
};
use openxr::SpaceLocationFlags;
use parking_lot::RwLock;
use zbus::{Connection, ObjectServer, interface};

use crate::{DbusConnection, PreFrameWait, get_time, nodes::spatial::Spatial};

use super::{AsyncTracked, ObjectHandle, SpatialRef, Tracked};

pub struct PlaySpacePlugin;
impl Plugin for PlaySpacePlugin {
	fn build(&self, app: &mut App) {
		app.add_systems(XrPreDestroySession, destroy_stage_space);
		app.add_systems(XrSessionCreated, create_stage_space);
		app.add_systems(PreFrameWait, update);
		app.add_systems(Startup, setup);
	}
}

fn setup(connection: Res<DbusConnection>, mut cmds: Commands) {
	let (spatial, spatial_handle) = SpatialRef::create(&connection, "/org/stardustxr/PlaySpace");
	// the OpenXR session might not exist quite yet
	let tracked = AsyncTracked::new(&connection, "/org/stardustxr/PlaySpace");
	let dbus_connection = connection.clone();
	let play_space_data = Arc::new(RwLock::default());
	tokio::task::spawn({
		let data = play_space_data.clone();
		async move {
			PlaySpaceBounds::create(&dbus_connection, data).await;
			dbus_connection
				.request_name("org.stardustxr.PlaySpace")
				.await
				.unwrap();
		}
	});
	cmds.insert_resource(PlaySpace {
		spatial,
		_spatial_handle: spatial_handle,
		tracked_handle: tracked,
		bounds: play_space_data,
	});
}

#[derive(Resource)]
struct StageSpace(XrSpace);
fn create_stage_space(session: Res<OxrSession>, mut cmds: Commands) {
	let space = session
		.create_reference_space(openxr::ReferenceSpaceType::STAGE, Transform::IDENTITY)
		.inspect_err(|err| error!("failed to create Stage XrSpace"))
		.ok();
	if let Some(space) = space {
		cmds.insert_resource(StageSpace(space.0));
	}
}
fn destroy_stage_space(session: Res<OxrSession>, mut cmds: Commands, stage: Res<StageSpace>) {
	session.destroy_space(stage.0);
	cmds.remove_resource::<StageSpace>();
}

/// TODO: impl this
fn update(
	session: Option<Res<OxrSession>>,
	stage: Option<Res<StageSpace>>,
	ref_space: Option<Res<XrPrimaryReferenceSpace>>,
	play_space: Res<PlaySpace>,
	state: Option<Res<OxrFrameState>>,
	pipelined: Option<Res<Pipelined>>,
) {
	let (Some(session), Some(stage), Some(ref_space), Some(state)) =
		(session, stage, ref_space, state)
	else {
		play_space.bounds.write().drain(..);
		play_space.tracked_handle.set_tracked(false);

		play_space
			.spatial
			.set_local_transform(Mat4::from_translation(vec3(0.0, -1.65, 0.0)));
		return;
	};
	let time = get_time(pipelined.is_some(), &state);
	let location = session
		.locate_space(&stage.0, &ref_space, time)
		.inspect_err(|err| error!("Error while Locating OpenXR Stage Space {err}"));
	if let Ok(location) = location {
		let is_tracked = location.location_flags.contains(
			SpaceLocationFlags::POSITION_VALID
				| SpaceLocationFlags::POSITION_TRACKED
				| SpaceLocationFlags::ORIENTATION_VALID
				| SpaceLocationFlags::ORIENTATION_TRACKED,
		);
		play_space.tracked_handle.set_tracked(is_tracked);
		if is_tracked {
			play_space
				.spatial
				.set_local_transform(Mat4::from_rotation_translation(
					location.pose.orientation.to_quat(),
					location.pose.position.to_vec3(),
				));
		}
	}
	// session.reference_space_bounds_rect(openxr::ReferenceSpaceType::STAGE);

	// if (World::has_bounds()
	// 	&& World::get_bounds_size().x != 0.0
	// 	&& World::get_bounds_size().y != 0.0)
	// {
	// 	let bounds = World::get_bounds_size();
	// 	vec![
	// 		((bounds.x).into(), (bounds.y).into()),
	// 		((bounds.x).into(), (-bounds.y).into()),
	// 		((-bounds.x).into(), (-bounds.y).into()),
	// 		((-bounds.x).into(), (bounds.y).into()),
	// 	]
	// } else {
	// 	vec![]
	// }
}

#[derive(Resource)]
pub struct PlaySpace {
	spatial: Arc<Spatial>,
	_spatial_handle: ObjectHandle<SpatialRef>,
	tracked_handle: AsyncTracked,
	bounds: Arc<RwLock<Vec<(f64, f64)>>>,
}
pub struct PlaySpaceBounds(Arc<RwLock<Vec<(f64, f64)>>>);
impl PlaySpaceBounds {
	pub async fn create(connection: &Connection, data: Arc<RwLock<Vec<(f64, f64)>>>) {
		connection
			.object_server()
			.at("/org/stardustxr/PlaySpace", Self(data))
			.await
			.unwrap();
	}
}
#[interface(name = "org.stardustxr.PlaySpace")]
impl PlaySpaceBounds {
	#[zbus(property)]
	fn bounds(&self) -> Vec<(f64, f64)> {
		self.0.read().clone()
	}
}

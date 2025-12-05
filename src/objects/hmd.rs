use super::{ObjectHandle, SpatialRef, input::mouse_pointer::FlatscreenCam};
use crate::{DbusConnection, PreFrameWait, get_time, nodes::spatial::Spatial};
use bevy::prelude::*;
use bevy_mod_openxr::{
	helper_traits::{ToQuat as _, ToVec3 as _},
	resources::{OxrFrameState, Pipelined},
	session::OxrSession,
};
use bevy_mod_xr::{
	session::{XrPreDestroySession, XrSessionCreated, session_running},
	spaces::{XrPrimaryReferenceSpace, XrSpace},
};
use openxr::SpaceLocationFlags;
use std::sync::Arc;

pub struct HmdPlugin;
impl Plugin for HmdPlugin {
	fn build(&self, app: &mut App) {
		app.add_systems(XrPreDestroySession, destroy_view_space);
		app.add_systems(XrSessionCreated, create_view_space);
		app.add_systems(PreFrameWait, update_xr.run_if(session_running));
		app.add_systems(PreFrameWait, update_flat.run_if(not(session_running)));
		app.add_systems(Startup, setup);
	}
}

fn setup(connection: Res<DbusConnection>, mut cmds: Commands) {
	let (spatial, _spatial_handle) = SpatialRef::create(&connection, "/org/stardustxr/HMD");
	let hmd = Hmd {
		spatial,
		_spatial_handle,
		space: None,
	};
	cmds.insert_resource(hmd);
}

fn create_view_space(session: Res<OxrSession>, mut hmd: ResMut<Hmd>) {
	let space = session
		.create_reference_space(openxr::ReferenceSpaceType::VIEW, Transform::IDENTITY)
		.inspect_err(|err| error!("failed to create View XrSpace"))
		.ok();
	hmd.space = space.map(|v| v.0);
}
fn destroy_view_space(session: Res<OxrSession>, mut cmds: Commands, mut hmd: ResMut<Hmd>) {
	let Some(space) = hmd.space.take() else {
		return;
	};
	session.destroy_space(space);
}

#[derive(Resource)]
struct Hmd {
	spatial: Arc<Spatial>,
	_spatial_handle: ObjectHandle<SpatialRef>,
	space: Option<XrSpace>,
}

fn update_flat(cam: Single<&GlobalTransform, With<FlatscreenCam>>, hmd: Res<Hmd>) {
	// this shouldn't be parented to anything, so global and local spaces should be the same
	hmd.spatial.set_local_transform(cam.compute_matrix());
}

fn update_xr(
	session: Option<Res<OxrSession>>,
	ref_space: Option<Res<XrPrimaryReferenceSpace>>,
	hmd: Res<Hmd>,
	state: Option<Res<OxrFrameState>>,
	pipelined: Option<Res<Pipelined>>,
) {
	let (Some(session), Some(view), Some(ref_space), Some(state)) =
		(session, hmd.space, ref_space, state)
	else {
		// tokio::task::spawn({
		// 	let handle = hmd.tracked_handle.clone();
		// 	async move {
		// 		handle.set_tracked(false);
		// 	}
		// });
		return;
	};
	let time = get_time(pipelined.is_some(), &state);
	let location = session
		.locate_space(&view, &ref_space, time)
		.inspect_err(|err| error!("Error while Locating OpenXR Stage Space {err}"));
	if let Ok(location) = location {
		let is_tracked = location
			.location_flags
			.contains(SpaceLocationFlags::POSITION_VALID | SpaceLocationFlags::POSITION_TRACKED)
			|| location.location_flags.contains(
				SpaceLocationFlags::ORIENTATION_VALID | SpaceLocationFlags::ORIENTATION_TRACKED,
			);
		// tokio::task::spawn({
		// 	let handle = play_space.tracked_handle.clone();
		// 	async move {
		// 		handle.set_tracked(is_tracked);
		// 	}
		// });
		if is_tracked {
			hmd.spatial
				.set_local_transform(Mat4::from_rotation_translation(
					location.pose.orientation.to_quat(),
					location.pose.position.to_vec3(),
				));
		}
	}
}

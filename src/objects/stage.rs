use crate::nodes::ProxyExt;
use crate::objects::DebugWrapper;
use crate::openxr_helpers::ConvertTimespec;
use crate::{DbusConnection, PreFrameWait, get_time};
use crate::{
	bevy_int::flatscreen_cam::FlatscreenCam, nodes::spatial::SpatialObject, objects::Tracked,
};
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
use gluon::ObjectRef;
use openxr::{Posef, ReferenceSpaceType, SpaceLocationFlags};
use stardust_xr_protocol::spatial::{Spatial, SpatialRef};
use stardust_xr_protocol::types::Timestamp;
use std::sync::Arc;

pub struct StagePlugin;
impl Plugin for StagePlugin {
	fn build(&self, app: &mut App) {
		app.add_systems(XrPreDestroySession, destroy_stage_space);
		app.add_systems(XrSessionCreated, create_stage_space);
		app.add_systems(PreFrameWait, update.run_if(session_running));
		app.add_systems(Startup, setup);
	}
}

fn setup(connection: Res<DbusConnection>, mut cmds: Commands) {
	let base_spatial = SpatialObject::new(None, Mat4::IDENTITY);
	let spatial = SpatialObject::new(Some(&base_spatial), Mat4::IDENTITY);
	let hmd = Stage {
		tracked: Tracked::new(
			SpatialRef::from_handler(spatial.get_ref()),
			|data, spatial, time| (None, false),
			false,
			"stardust-stage",
			DebugWrapper((None, base_spatial.clone(), None, spatial.clone())),
		)
		.expect("failed to create Tracked for Stage"),
		spatial,
		base_spatial,
	};
	cmds.insert_resource(hmd);
}

fn dyn_tracking(
	DebugWrapper((base_space, base_spatial, stage_space, spatial)): &DebugWrapper<(
		Option<openxr::Space>,
		ObjectRef<SpatialObject>,
		Option<openxr::Space>,
		ObjectRef<SpatialObject>,
	)>,
	reference_spatial: &Spatial,
	time: Timestamp,
) -> (Option<stardust_xr_protocol::types::Posef>, bool) {
	let Some(reference_spatial) = reference_spatial.owned() else {
		return (None, false);
	};
	if let Some(base_space) = base_space.as_ref()
		&& let Some(stage_space) = stage_space.as_ref()
	{
		let Some(time) = stage_space.instance().timestamp_to_xr(time) else {
			return (None, false);
		};
		let Ok(location) = stage_space.locate(base_space, time) else {
			return (None, false);
		};
		let valid = location
			.location_flags
			.contains(SpaceLocationFlags::POSITION_VALID | SpaceLocationFlags::ORIENTATION_VALID);
		let tracked = location.location_flags.contains(
			SpaceLocationFlags::POSITION_TRACKED | SpaceLocationFlags::ORIENTATION_TRACKED,
		);
		let mat = crate::nodes::spatial::Spatial::space_to_space_matrix(
			Some(base_spatial),
			Some(&reference_spatial),
		);
		let (_, rot, _) = mat.to_scale_rotation_translation();
		let pose = valid.then(|| stardust_xr_protocol::types::Posef {
			orientation: (rot * location.pose.orientation.to_quat()).into(),
			position: mat
				.transform_point3(location.pose.position.to_vec3())
				.into(),
		});
		(pose, tracked)
	} else {
		let mat = crate::nodes::spatial::Spatial::space_to_space_matrix(
			Some(base_spatial),
			Some(spatial),
		);
		let (_, rot, pos) = mat.to_scale_rotation_translation();
		(
			Some(stardust_xr_protocol::types::Posef {
				position: pos.into(),
				orientation: rot.into(),
			}),
			true,
		)
	}
}

fn create_stage_space(session: Res<OxrSession>, mut hmd: ResMut<Stage>) {
	let stage_space = (**session)
		.create_reference_space(ReferenceSpaceType::STAGE, Posef::IDENTITY)
		.inspect_err(|err| error!("failed to create openxr stage space: {err}"))
		.ok();
	let local_space = (**session)
		.create_reference_space(ReferenceSpaceType::LOCAL, Posef::IDENTITY)
		.inspect_err(|err| error!("failed to create openxr local space: {err}"))
		.ok();
	let mut v = hmd.tracked.get_mut_data_blocking();
	v.0.0 = local_space;
	v.2 = stage_space;
}
fn destroy_stage_space(session: Res<OxrSession>, mut cmds: Commands, mut hmd: ResMut<Stage>) {
	let mut v = hmd.tracked.get_mut_data_blocking();
	v.0.0.take();
	v.2.take();
}

#[derive(Resource)]
struct Stage {
	spatial: gluon::ObjectRef<SpatialObject>,
	base_spatial: gluon::ObjectRef<SpatialObject>,
	tracked: Tracked<
		DebugWrapper<(
			Option<openxr::Space>,
			ObjectRef<SpatialObject>,
			Option<openxr::Space>,
			ObjectRef<SpatialObject>,
		)>,
	>,
}

fn update(
	session: Option<Res<OxrSession>>,
	ref_space: Option<Res<XrPrimaryReferenceSpace>>,
	stage: Res<Stage>,
	state: Option<Res<OxrFrameState>>,
	pipelined: Option<Res<Pipelined>>,
) {
	let (Some(session), Some(ref_space), Some(state)) = (session, ref_space, state) else {
		stage.tracked.tracked_blocking(false);
		return;
	};
	let time = get_time(pipelined.is_some(), &state);

	let state = stage.tracked.get_data_blocking();
	let Some(base_space) = &state.0.0 else {
		return;
	};
	let Some(stage_space) = &state.2 else {
		return;
	};
	let pose = session.locate_space(
		&unsafe { XrSpace::from_raw(base_space.as_raw().into_raw()) },
		&ref_space,
		time,
	);
	if let Ok(pose) = pose
		&& pose.location_flags.contains(
			SpaceLocationFlags::POSITION_TRACKED | SpaceLocationFlags::ORIENTATION_TRACKED,
		) {
		stage
			.base_spatial
			.set_local_transform(Mat4::from_rotation_translation(
				pose.pose.orientation.to_quat(),
				pose.pose.position.to_vec3(),
			));
	}
	let location = stage_space
		.locate(base_space, time)
		.inspect_err(|err| error!("Error while Locating OpenXR Stage Space {err}"));
	if let Ok(location) = location {
		let is_tracked = location
			.location_flags
			.contains(SpaceLocationFlags::POSITION_VALID | SpaceLocationFlags::POSITION_TRACKED)
			|| location.location_flags.contains(
				SpaceLocationFlags::ORIENTATION_VALID | SpaceLocationFlags::ORIENTATION_TRACKED,
			);
		stage.tracked.tracked_blocking(is_tracked);
		if is_tracked {
			stage
				.spatial
				.set_local_transform(Mat4::from_rotation_translation(
					location.pose.orientation.to_quat(),
					location.pose.position.to_vec3(),
				));
		}
	}
}

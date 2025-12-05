use bevy::{
	app::{Plugin, Update},
	ecs::{
		resource::Resource,
		schedule::{Condition, IntoScheduleConfigs, common_conditions::resource_exists},
		system::{Commands, Res, ResMut},
	},
	transform::components::Transform,
};
use bevy_mod_openxr::{
	helper_traits::ToTransform as _,
	poll_events::{OxrEventHandlerExt, OxrEventIn},
	resources::OxrFrameState,
	session::OxrSession,
};
use bevy_mod_xr::{session::XrSessionCreated, spaces::XrPrimaryReferenceSpace};
use glam::{Quat, Vec3};
use openxr::ReferenceSpaceType;

pub struct TrackingOffsetPlugin;

impl Plugin for TrackingOffsetPlugin {
	fn build(&self, app: &mut bevy::app::App) {
		app.add_oxr_event_handler(reset_offset);
		app.add_systems(XrSessionCreated, |mut cmds: Commands| {
			cmds.insert_resource(OffsetTag);
		});
		app.add_systems(
			Update,
			offset.run_if(resource_exists::<OffsetTag>.and(resource_exists::<OxrFrameState>)),
		);
	}
}

#[derive(Resource)]
struct OffsetTag;

fn reset_offset(
	oxr_event: OxrEventIn,
	mut ref_space: ResMut<XrPrimaryReferenceSpace>,
	session: Res<OxrSession>,
) {
	if let openxr::Event::ReferenceSpaceChangePending(v) = *oxr_event
		&& v.reference_space_type() == ReferenceSpaceType::LOCAL
	{
		let space = session
			.create_reference_space(ReferenceSpaceType::LOCAL, Transform::IDENTITY)
			.unwrap();
		session.destroy_space(ref_space.0.0).unwrap();
		ref_space.0 = space;
	}
}

fn offset(
	session: Res<OxrSession>,
	state: Res<OxrFrameState>,
	mut primary_ref_space: ResMut<XrPrimaryReferenceSpace>,
	mut cmds: Commands,
) {
	cmds.remove_resource::<OffsetTag>();
	let local = session
		.create_reference_space(ReferenceSpaceType::LOCAL, Transform::IDENTITY)
		.unwrap();
	let view = session
		.create_reference_space(ReferenceSpaceType::VIEW, Transform::IDENTITY)
		.unwrap();
	let view_pose = session
		.locate_space(&view, &local, state.predicted_display_time)
		.unwrap()
		.pose
		.to_transform();
	let offset = view_pose.with_rotation(Quat::from_axis_angle(
		Vec3::Y,
		view_pose.rotation.to_euler(glam::EulerRot::XYZ).1,
	));
	let offset = Transform::from_matrix(offset.compute_matrix());
	let local_offset = session
		.create_reference_space(ReferenceSpaceType::LOCAL, offset)
		.unwrap();
	session.destroy_space(primary_ref_space.0.0).unwrap();
	primary_ref_space.0 = local_offset;
}

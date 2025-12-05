use bevy::{
	app::Plugin,
	core_pipeline::core_3d::Camera3d,
	ecs::{
		component::Component,
		system::{Commands, Res},
	},
	render::camera::{PerspectiveProjection, Projection},
	transform::components::Transform,
};
use bevy_mod_openxr::session::OxrSession;
use bevy_mod_xr::session::XrSessionCreated;
use openxr::ReferenceSpaceType;

pub struct SpectatorCameraPlugin;

impl Plugin for SpectatorCameraPlugin {
	fn build(&self, app: &mut bevy::app::App) {
		app.add_systems(XrSessionCreated, create);
	}
}

#[derive(Component)]
#[require(Camera3d)]
struct SpectatorCam;

fn create(session: Res<OxrSession>, mut cmds: Commands) {
	cmds.spawn((
		SpectatorCam,
		session
			.create_reference_space(ReferenceSpaceType::VIEW, Transform::IDENTITY)
			.unwrap()
			.0,
		Projection::Perspective(PerspectiveProjection {
			fov: 100f32.to_radians(),
			..Default::default()
		}),
	));
}

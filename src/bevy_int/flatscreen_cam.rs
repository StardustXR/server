use bevy::{
	app::{Plugin, Startup, Update},
	core_pipeline::core_3d::Camera3d,
	ecs::{
		component::Component,
		name::Name,
		query::With,
		system::{Commands, Res, Single},
	},
	input::{
		ButtonInput,
		keyboard::KeyCode,
		mouse::{MouseButton, MouseMotion},
	},
	math::{EulerRot, Quat, Vec3},
	prelude::EventReader,
	render::camera::{PerspectiveProjection, Projection},
	time::Time,
	transform::components::Transform,
};

/// A first-person fly camera for flatscreen mode.
///
/// Hold `Shift + RightClick` to look around with the mouse and move with
/// `WASD` (plus `Q`/`E` for down/up). Independent of the Stardust input-method
/// / mouse-pointer pipeline.
pub struct FlatscreenCamPlugin;

impl Plugin for FlatscreenCamPlugin {
	fn build(&self, app: &mut bevy::app::App) {
		app.add_systems(Startup, spawn_cam);
		app.add_systems(Update, fly_cam_controls);
	}
}

#[derive(Component)]
#[require(Camera3d)]
pub struct FlatscreenCam;

fn spawn_cam(mut cmds: Commands) {
	cmds.spawn((
		FlatscreenCam,
		Name::new("Flatscreen Camera"),
		Projection::Perspective(PerspectiveProjection {
			fov: 100f32.to_radians(),
			..Default::default()
		}),
		Transform::from_xyz(0.0, 1.6, 0.0),
	));
}

fn fly_cam_controls(
	mut cam: Single<&mut Transform, With<FlatscreenCam>>,
	mouse_buttons: Res<ButtonInput<MouseButton>>,
	keyboard_buttons: Res<ButtonInput<KeyCode>>,
	mut motion: EventReader<MouseMotion>,
	time: Res<Time>,
) {
	if !(keyboard_buttons.pressed(KeyCode::ShiftLeft)
		&& mouse_buttons.pressed(MouseButton::Right))
	{
		// Drain motion events so they don't accumulate for the next time the
		// user engages look mode.
		motion.clear();
		return;
	}

	let (mut yaw, mut pitch, _) = cam.rotation.to_euler(EulerRot::YXZ);
	for e in motion.read() {
		let scale = -0.003;
		pitch += e.delta.y * scale;
		yaw += e.delta.x * scale;
	}
	// Clamp pitch to avoid flipping past the poles.
	pitch = pitch.clamp(-std::f32::consts::FRAC_PI_2 + 0.001, std::f32::consts::FRAC_PI_2 - 0.001);
	cam.rotation = Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch);

	let mut move_vec = Vec3::ZERO;
	move_vec.x += keyboard_buttons.pressed(KeyCode::KeyD) as u32 as f32;
	move_vec.x -= keyboard_buttons.pressed(KeyCode::KeyA) as u32 as f32;
	move_vec.z += keyboard_buttons.pressed(KeyCode::KeyS) as u32 as f32;
	move_vec.z -= keyboard_buttons.pressed(KeyCode::KeyW) as u32 as f32;
	move_vec.y += keyboard_buttons.pressed(KeyCode::KeyE) as u32 as f32;
	move_vec.y -= keyboard_buttons.pressed(KeyCode::KeyQ) as u32 as f32;

	let speed = if keyboard_buttons.pressed(KeyCode::ControlLeft) {
		10.0
	} else {
		3.0
	};
	let move_vec = cam.rotation * move_vec.normalize_or_zero();
	cam.translation += move_vec * time.delta_secs() * speed;
}

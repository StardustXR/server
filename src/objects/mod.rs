#![allow(unused)]

use crate::{
	core::client::INTERNAL_CLIENT,
	nodes::{
		fields::{Field, Shape, EXPORTED_FIELDS},
		spatial::{Spatial, EXPORTED_SPATIALS},
		Node,
	},
};
use glam::{vec3, Mat4};
use input::{
	eye_pointer::EyePointer, mouse_pointer::MousePointer, sk_controller::SkController,
	sk_hand::SkHand,
};
use play_space::PlaySpaceBounds;
use std::sync::Arc;
use stereokit_rust::{
	sk::{DisplayMode, MainThreadToken, Sk},
	system::{Handed, Input, Key, World},
	util::Device,
};
use tokio::task::AbortHandle;
use zbus::{interface, Connection};

pub mod input;
pub mod play_space;

enum Inputs {
	XR {
		controller_left: SkController,
		controller_right: SkController,
		hand_left: SkHand,
		hand_right: SkHand,
		eye_pointer: Option<EyePointer>,
	},
	MousePointer(MousePointer),
	// Controllers((SkController, SkController)),
	Hands {
		left: SkHand,
		right: SkHand,
	},
}

pub struct ServerObjects {
	connection: Connection,
	hmd: (Arc<Spatial>, AbortHandle),
	play_space: Option<(Arc<Spatial>, AbortHandle)>,
	inputs: Inputs,
}
impl ServerObjects {
	pub fn new(connection: Connection, sk: &Sk) -> ServerObjects {
		let hmd = SpatialRef::create(&connection, "/org/stardustxr/HMD");

		let play_space = (World::has_bounds()
			&& World::get_bounds_size().x != 0.0
			&& World::get_bounds_size().y != 0.0)
			.then(|| SpatialRef::create(&connection, "/org/stardustxr/PlaySpace"));
		if play_space.is_some() {
			let dbus_connection = connection.clone();
			tokio::task::spawn(async move {
				PlaySpaceBounds::create(&dbus_connection).await;
				dbus_connection
					.request_name("org.stardustxr.PlaySpace")
					.await
					.unwrap();
			});
		}

		let inputs = if sk.get_active_display_mode() == DisplayMode::MixedReality {
			Inputs::XR {
				controller_left: SkController::new(Handed::Left).unwrap(),
				controller_right: SkController::new(Handed::Right).unwrap(),
				hand_left: SkHand::new(Handed::Left).unwrap(),
				hand_right: SkHand::new(Handed::Right).unwrap(),
				eye_pointer: Device::has_eye_gaze()
					.then(EyePointer::new)
					.transpose()
					.unwrap(),
			}
		} else {
			Inputs::MousePointer(MousePointer::new().unwrap())
		};

		ServerObjects {
			connection,
			hmd,
			play_space,
			inputs,
		}
	}

	pub fn update(&mut self, sk: &Sk, token: &MainThreadToken) {
		let hmd_pose = Input::get_head();
		self.hmd
			.0
			.set_local_transform(Mat4::from_scale_rotation_translation(
				vec3(1.0, 1.0, 1.0),
				hmd_pose.orientation.into(),
				hmd_pose.position.into(),
			));

		if let Some(play_space) = self.play_space.as_ref() {
			let pose = World::get_bounds_pose();
			play_space
				.0
				.set_local_transform(Mat4::from_rotation_translation(
					pose.orientation.into(),
					pose.position.into(),
				));
		}

		if sk.get_active_display_mode() != DisplayMode::MixedReality {
			if Input::key(Key::F6).is_just_inactive() {
				self.inputs = Inputs::MousePointer(MousePointer::new().unwrap());
			}
			// if Input::key(Key::F7).is_just_inactive() {
			// 	self.inputs = Inputs::Controllers((
			// 		SkController::new(Handed::Left).unwrap(),
			// 		SkController::new(Handed::Right).unwrap(),
			// 	));
			// }
			if Input::key(Key::F8).is_just_inactive() {
				self.inputs = Inputs::Hands {
					left: SkHand::new(Handed::Left).unwrap(),
					right: SkHand::new(Handed::Right).unwrap(),
				};
			}
		}

		match &mut self.inputs {
			Inputs::XR {
				controller_left,
				controller_right,
				hand_left,
				hand_right,
				eye_pointer,
			} => {
				controller_left.update(token);
				controller_right.update(token);
				hand_left.update(sk, token);
				hand_right.update(sk, token);
				if let Some(eye_pointer) = eye_pointer {
					eye_pointer.update();
				}
			}
			Inputs::MousePointer(mouse_pointer) => mouse_pointer.update(),
			// Inputs::Controllers((left, right)) => {
			// 	left.update(token);
			// 	right.update(token);
			// }
			Inputs::Hands { left, right } => {
				left.update(sk, token);
				right.update(sk, token);
			}
		}
	}
}
impl Drop for ServerObjects {
	fn drop(&mut self) {
		self.hmd.1.abort();
		if let Some((_, play_space)) = self.play_space.take() {
			play_space.abort();
		}
	}
}

pub struct SpatialRef(u64);
impl SpatialRef {
	pub fn create(connection: &Connection, path: &str) -> (Arc<Spatial>, AbortHandle) {
		let node = Arc::new(Node::generate(&INTERNAL_CLIENT, false));
		let spatial = Spatial::add_to(&node, None, Mat4::IDENTITY, false);
		let uid: u64 = rand::random();
		EXPORTED_SPATIALS.lock().insert(uid, node.clone());

		let connection = connection.clone();
		let path = path.to_string();
		(
			spatial,
			tokio::task::spawn(async move {
				connection
					.object_server()
					.at(path, Self(uid))
					.await
					.unwrap();
			})
			.abort_handle(),
		)
	}
}
#[interface(name = "org.stardustxr.SpatialRef")]
impl SpatialRef {
	#[zbus(property)]
	fn uid(&self) -> u64 {
		self.0
	}
}

pub struct FieldRef(u64);
impl FieldRef {
	pub fn create(connection: &Connection, path: &str, shape: Shape) -> (Arc<Field>, AbortHandle) {
		let node = Arc::new(Node::generate(&INTERNAL_CLIENT, false));
		Spatial::add_to(&node, None, Mat4::IDENTITY, false);
		let field = Field::add_to(&node, shape).unwrap();
		let uid: u64 = rand::random();
		EXPORTED_FIELDS.lock().insert(uid, node.clone());

		let connection = connection.clone();
		let path = path.to_string();
		(
			field,
			tokio::task::spawn(async move {
				connection
					.object_server()
					.at(path, Self(uid))
					.await
					.unwrap();
			})
			.abort_handle(),
		)
	}
}
#[interface(name = "org.stardustxr.FieldRef")]
impl FieldRef {
	#[zbus(property)]
	fn uid(&self) -> u64 {
		self.0
	}
}

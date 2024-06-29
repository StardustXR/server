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
use std::sync::Arc;
use stereokit_rust::{
	sk::{DisplayMode, MainThreadToken, Sk},
	system::{Handed, Input, Key, World},
	util::Device,
};
use zbus::{interface, Connection};

pub mod input;
pub mod play_space;

enum Inputs {
	XR {
		controllers: (SkController, SkController),
		hands: (SkHand, SkHand),
		eye_pointer: Option<EyePointer>,
	},
	MousePointer(MousePointer),
	Controllers((SkController, SkController)),
	Hands((SkHand, SkHand)),
}

pub struct ServerObjects {
	hmd: Arc<Spatial>,
	play_space: Option<Arc<Spatial>>,
	inputs: Inputs,
}
impl ServerObjects {
	pub fn new(sk: &Sk, hmd: Arc<Spatial>, play_space: Option<Arc<Spatial>>) -> ServerObjects {
		let inputs = if sk.get_active_display_mode() == DisplayMode::MixedReality {
			Inputs::XR {
				controllers: (
					SkController::new(Handed::Left).unwrap(),
					SkController::new(Handed::Right).unwrap(),
				),
				hands: (
					SkHand::new(Handed::Left).unwrap(),
					SkHand::new(Handed::Right).unwrap(),
				),
				eye_pointer: Device::has_eye_gaze()
					.then(EyePointer::new)
					.transpose()
					.unwrap(),
			}
		} else {
			Inputs::MousePointer(MousePointer::new().unwrap())
		};

		ServerObjects {
			hmd,
			play_space,
			inputs,
		}
	}

	pub fn update(&mut self, sk: &Sk, token: &MainThreadToken) {
		let hmd_pose = Input::get_head();
		self.hmd
			.set_local_transform(Mat4::from_scale_rotation_translation(
				vec3(1.0, 1.0, 1.0),
				hmd_pose.orientation.into(),
				hmd_pose.position.into(),
			));

		if let Some(play_space) = self.play_space.as_ref() {
			let pose = World::get_bounds_pose();
			play_space.set_local_transform(Mat4::from_rotation_translation(
				pose.orientation.into(),
				pose.position.into(),
			));
		}

		if sk.get_active_display_mode() != DisplayMode::MixedReality {
			if Input::key(Key::F6).is_just_inactive() {
				self.inputs = Inputs::MousePointer(MousePointer::new().unwrap());
			}
			if Input::key(Key::F7).is_just_inactive() {
				self.inputs = Inputs::Controllers((
					SkController::new(Handed::Left).unwrap(),
					SkController::new(Handed::Right).unwrap(),
				));
			}
			if Input::key(Key::F8).is_just_inactive() {
				self.inputs = Inputs::Hands((
					SkHand::new(Handed::Left).unwrap(),
					SkHand::new(Handed::Right).unwrap(),
				));
			}
		}

		match &mut self.inputs {
			Inputs::XR {
				controllers: (left_controller, right_controller),
				hands: (left_hand, right_hand),
				eye_pointer,
			} => {
				left_hand.update(sk, token);
				right_hand.update(sk, token);
				left_controller.update(token);
				right_controller.update(token);
				if let Some(eye_pointer) = eye_pointer {
					eye_pointer.update();
				}
			}
			Inputs::MousePointer(mouse_pointer) => mouse_pointer.update(),
			Inputs::Controllers((left, right)) => {
				left.update(token);
				right.update(token);
			}
			Inputs::Hands((left, right)) => {
				left.update(sk, token);
				right.update(sk, token);
			}
		}
	}
}

pub struct SpatialRef(u64);
impl SpatialRef {
	pub async fn create(connection: &Connection, path: &str) -> Arc<Node> {
		let node = Arc::new(Node::generate(&INTERNAL_CLIENT, false));
		Spatial::add_to(&node, None, Mat4::IDENTITY, false);
		let uid: u64 = rand::random();
		EXPORTED_SPATIALS.lock().insert(uid, node.clone());
		connection
			.object_server()
			.at(path, Self(uid))
			.await
			.unwrap();
		node
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
	pub async fn create(connection: &Connection, path: &str, shape: Shape) -> Arc<Node> {
		let node = Arc::new(Node::generate(&INTERNAL_CLIENT, false));
		Spatial::add_to(&node, None, Mat4::IDENTITY, false);
		Field::add_to(&node, shape).unwrap();
		let uid: u64 = rand::random();
		EXPORTED_FIELDS.lock().insert(uid, node.clone());
		connection
			.object_server()
			.at(path, Self(uid))
			.await
			.unwrap();
		node
	}
}
#[interface(name = "org.stardustxr.FieldRef")]
impl FieldRef {
	#[zbus(property)]
	fn uid(&self) -> u64 {
		self.0
	}
}

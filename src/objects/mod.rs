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
	system::{Handed, Input, World},
	util::Device,
};
use zbus::{interface, Connection};

pub mod input;
pub mod play_space;

pub struct ServerObjects {
	hmd: Arc<Spatial>,
	play_space: Option<Arc<Spatial>>,
	mouse_pointer: Option<MousePointer>,
	hands: Option<(SkHand, SkHand)>,
	controllers: Option<(SkController, SkController)>,
	eye_pointer: Option<EyePointer>,
}
impl ServerObjects {
	pub fn new(
		intentional_flatscreen: bool,
		sk: &Sk,
		hmd: Arc<Spatial>,
		play_space: Option<Arc<Spatial>>,
	) -> ServerObjects {
		ServerObjects {
			hmd,
			play_space,
			mouse_pointer: intentional_flatscreen
				.then(MousePointer::new)
				.transpose()
				.unwrap(),
			hands: (!intentional_flatscreen)
				.then(|| {
					let left = SkHand::new(Handed::Left).ok();
					let right = SkHand::new(Handed::Right).ok();
					left.zip(right)
				})
				.flatten(),
			controllers: (!intentional_flatscreen)
				.then(|| {
					let left = SkController::new(Handed::Left).ok();
					let right = SkController::new(Handed::Right).ok();
					left.zip(right)
				})
				.flatten(),
			eye_pointer: (sk.get_active_display_mode() == DisplayMode::MixedReality
				&& Device::has_eye_gaze())
			.then(EyePointer::new)
			.transpose()
			.unwrap(),
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

		if let Some(mouse_pointer) = self.mouse_pointer.as_mut() {
			mouse_pointer.update();
		}
		if let Some((left_hand, right_hand)) = self.hands.as_mut() {
			left_hand.update(sk, token);
			right_hand.update(sk, token);
		}
		if let Some((left_controller, right_controller)) = self.controllers.as_mut() {
			left_controller.update(token);
			right_controller.update(token);
		}
		if let Some(eye_pointer) = self.eye_pointer.as_ref() {
			eye_pointer.update();
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

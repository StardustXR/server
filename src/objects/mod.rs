#![allow(unused)]

use crate::{
	core::client::INTERNAL_CLIENT,
	nodes::{
		fields::{Field, Shape, EXPORTED_FIELDS},
		spatial::{Spatial, EXPORTED_SPATIALS},
		Node, OwnedNode,
	},
};
use glam::{vec3, Mat4};
use input::{
	eye_pointer::EyePointer, mouse_pointer::MousePointer, sk_controller::SkController,
	sk_hand::SkHand,
};
use play_space::PlaySpaceBounds;
use stardust_xr::schemas::dbus::object_registry::ObjectRegistry;
use std::{marker::PhantomData, sync::Arc};
use stereokit_rust::{
	material::Material,
	sk::{DisplayMode, MainThreadToken, Sk},
	system::{Handed, Input, Key, World},
	util::Device,
};
use zbus::{interface, object_server::Interface, zvariant::OwnedObjectPath, Connection};

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
	hmd: (Arc<Spatial>, ObjectHandle<SpatialRef>),
	play_space: Option<(Arc<Spatial>, ObjectHandle<SpatialRef>)>,
	hand_materials: [Material; 2],
	inputs: Inputs,
	disable_controllers: bool,
	disable_hands: bool,
}
impl ServerObjects {
	pub fn new(
		connection: Connection,
		sk: &Sk,
		hand_materials: [Material; 2],
		disable_controllers: bool,
		disable_hands: bool,
	) -> ServerObjects {
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

		tokio::task::spawn({
			let connection = connection.clone();
			async move {
				connection
					.request_name("org.stardustxr.Controllers")
					.await
					.unwrap();
				connection
					.request_name("org.stardustxr.Hands")
					.await
					.unwrap();
			}
		});

		let inputs = if sk.get_active_display_mode() == DisplayMode::MixedReality {
			Inputs::XR {
				controller_left: SkController::new(&connection, Handed::Left).unwrap(),
				controller_right: SkController::new(&connection, Handed::Right).unwrap(),
				hand_left: SkHand::new(&connection, Handed::Left).unwrap(),
				hand_right: SkHand::new(&connection, Handed::Right).unwrap(),
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
			hand_materials,
			inputs,
			disable_controllers,
			disable_hands,
		}
	}

	pub fn update(
		&mut self,
		sk: &Sk,
		token: &MainThreadToken,
		dbus_connection: &Connection,
		object_registry: &ObjectRegistry,
	) {
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

		#[allow(clippy::collapsible_if)]
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
			// if Input::key(Key::F8).is_just_inactive() {
			// 	self.inputs = Inputs::Hands {
			// 		left: SkHand::new(&self.connection, Handed::Left).unwrap(),
			// 		right: SkHand::new(&self.connection, Handed::Right).unwrap(),
			// 	};
			// }
		}

		match &mut self.inputs {
			Inputs::XR {
				controller_left,
				controller_right,
				hand_left,
				hand_right,
				eye_pointer,
			} => {
				if !self.disable_controllers {
					controller_left.update(token);
					controller_right.update(token);
				}
				Input::hand_visible(Handed::Left, !self.disable_hands);
				Input::hand_visible(Handed::Right, !self.disable_hands);
				if !self.disable_hands {
					hand_left.update(sk, token, &mut self.hand_materials[0]);
					hand_right.update(sk, token, &mut self.hand_materials[1]);
				}
				if let Some(eye_pointer) = eye_pointer {
					eye_pointer.update();
				}
			}
			Inputs::MousePointer(mouse_pointer) => {
				mouse_pointer.update(dbus_connection, object_registry)
			}
			// Inputs::Controllers((left, right)) => {
			// 	left.update(token);
			// 	right.update(token);
			// }
			Inputs::Hands { left, right } => {
				left.update(sk, token, &mut self.hand_materials[0]);
				right.update(sk, token, &mut self.hand_materials[1]);
			}
		}
	}
}

pub struct ObjectHandle<I: Interface>(Connection, OwnedObjectPath, PhantomData<I>);
impl<I: Interface> Drop for ObjectHandle<I> {
	fn drop(&mut self) {
		let connection = self.0.clone();
		let object_path = self.1.clone();
		tokio::task::spawn(async move {
			connection.object_server().remove::<I, _>(object_path);
		});
	}
}

pub struct SpatialRef(u64, OwnedNode);
impl SpatialRef {
	pub fn create(connection: &Connection, path: &str) -> (Arc<Spatial>, ObjectHandle<SpatialRef>) {
		let node = OwnedNode(Arc::new(Node::generate(&INTERNAL_CLIENT, false)));
		let spatial = Spatial::add_to(&node.0, None, Mat4::IDENTITY, false);
		let uid: u64 = rand::random();
		EXPORTED_SPATIALS.lock().insert(uid, node.0.clone());

		tokio::task::spawn({
			let connection = connection.clone();
			let path = path.to_string();
			async move {
				connection
					.object_server()
					.at(path, Self(uid, node))
					.await
					.unwrap();
			}
		});
		(
			spatial,
			ObjectHandle(
				connection.clone(),
				OwnedObjectPath::try_from(path.to_string()).unwrap(),
				PhantomData,
			),
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

pub struct FieldRef(u64, OwnedNode);
impl FieldRef {
	pub fn create(
		connection: &Connection,
		path: &str,
		shape: Shape,
	) -> (Arc<Field>, ObjectHandle<FieldRef>) {
		let node = OwnedNode(Arc::new(Node::generate(&INTERNAL_CLIENT, false)));
		Spatial::add_to(&node.0, None, Mat4::IDENTITY, false);
		let field = Field::add_to(&node.0, shape).unwrap();
		let uid: u64 = rand::random();
		EXPORTED_FIELDS.lock().insert(uid, node.0.clone());

		tokio::task::spawn({
			let connection = connection.clone();
			let path = path.to_string();
			async move {
				connection
					.object_server()
					.at(path, Self(uid, node))
					.await
					.unwrap();
			}
		});
		(
			field,
			ObjectHandle(
				connection.clone(),
				OwnedObjectPath::try_from(path.to_string()).unwrap(),
				PhantomData,
			),
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

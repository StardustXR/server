#![allow(unused)]

use crate::{
	core::client::INTERNAL_CLIENT,
	nodes::{
		fields::{Field, Shape, EXPORTED_FIELDS},
		spatial::{Spatial, EXPORTED_SPATIALS},
		Node, OwnedNode,
	},
	TOKIO,
};
use bevy::prelude::Resource;
use bevy_mod_openxr::helper_traits::{ToQuat, ToVec3};
use glam::{vec3, Mat4};
use input::{
	eye_pointer::EyePointer, mouse_pointer::MousePointer, sk_controller::SkController,
	sk_hand::SkHand,
};
use openxr::SpaceLocationFlags;
use play_space::PlaySpaceBounds;
use stardust_xr::schemas::dbus::object_registry::ObjectRegistry;
use std::{marker::PhantomData, sync::Arc};
use zbus::{interface, object_server::Interface, zvariant::OwnedObjectPath, Connection};

pub mod input;
pub mod play_space;

pub(crate) enum Inputs {
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

struct ControllerInput {
	pose: openxr::Space,
	select: openxr::Action<f32>,
	grab: openxr::Action<f32>,
	scroll: openxr::Action<openxr::Vector2f>,
}

#[derive(Resource)]
pub struct ServerObjects {
	connection: Connection,
	hmd: (Arc<Spatial>, ObjectHandle<SpatialRef>),
	play_space: Option<(Arc<Spatial>, ObjectHandle<SpatialRef>)>,
	inputs: Option<Inputs>,
	view_space: Option<openxr::Space>,
	ref_space: Option<openxr::Space>,
}

pub struct TrackingRefs {
	view_space: openxr::Space,
	ref_space: openxr::Space,
	left_controller_space: openxr::Space,
	right_controller_space: openxr::Space,
	left_hand_tracker: Option<openxr::HandTracker>,
	right_hand_tracker: Option<openxr::HandTracker>,
}

impl ServerObjects {
	pub fn new(connection: Connection) -> ServerObjects {
		let hmd = SpatialRef::create(&connection, "/org/stardustxr/HMD");

		// TODO: implement in bevy_mod_openxr
		// let play_space = (World::has_bounds()
		// 	&& World::get_bounds_size().x != 0.0
		// 	&& World::get_bounds_size().y != 0.0)
		// 	.then(|| SpatialRef::create(&connection, "/org/stardustxr/PlaySpace"));
		// let play_space = None;
		// if play_space.is_some() {
		// 	let dbus_connection = connection.clone();
		// 	TOKIO.spawn(async move {
		// 		PlaySpaceBounds::create(&dbus_connection).await;
		// 		dbus_connection
		// 			.request_name("org.stardustxr.PlaySpace")
		// 			.await
		// 			.unwrap();
		// 	});
		// }

		TOKIO.spawn({
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

		// let inputs = if sk.get_active_display_mode() == DisplayMode::MixedReality {
		// 	Inputs::XR {
		// 		controller_left: SkController::new(&connection, Handed::Left).unwrap(),
		// 		controller_right: SkController::new(&connection, Handed::Right).unwrap(),
		// 		hand_left: SkHand::new(&connection, Handed::Left).unwrap(),
		// 		hand_right: SkHand::new(&connection, Handed::Right).unwrap(),
		// 		// TODO: implement in bevy_mod_openxr
		// 		eye_pointer: false.then(EyePointer::new).transpose().unwrap(),
		// 	}
		// } else {
		// 	Inputs::MousePointer(MousePointer::new().unwrap())
		// };

		ServerObjects {
			connection,
			hmd,
			play_space: None,
			inputs: None,
			ref_space: None,
			view_space: None,
		}
	}

	pub fn update(
		&mut self,
		session: Option<&openxr::Session<openxr::AnyGraphics>>,
		time: Option<openxr::Time>,
	) {
		if let (Some(session), Some(ref_space), Some(time)) =
			(session, self.ref_space.as_ref(), time)
		{
			'hmd: {
				if let Some(view) = self.view_space.as_ref() {
					let hmd_pose = match view.locate(ref_space, time) {
						Ok(v) => v,
						Err(err) => {
							tracing::error!("error while locating hmd: {err}");
							break 'hmd;
						}
					};
					if hmd_pose.location_flags.contains(
						SpaceLocationFlags::POSITION_TRACKED
							| SpaceLocationFlags::ORIENTATION_TRACKED,
					) {
						self.hmd
							.0
							.set_local_transform(Mat4::from_scale_rotation_translation(
								vec3(1.0, 1.0, 1.0),
								hmd_pose.pose.orientation.to_quat(),
								hmd_pose.pose.position.to_vec3(),
							));
					}
				}
			}
		}

		// if let Some(play_space) = self.play_space.as_ref() {
		// 	let pose = World::get_bounds_pose();
		// 	play_space
		// 		.0
		// 		.set_local_transform(Mat4::from_rotation_translation(
		// 			pose.orientation.into(),
		// 			pose.position.into(),
		// 		));
		// }

		// if sk.get_active_display_mode() != DisplayMode::MixedReality {
		// 	if Input::key(Key::F6).is_just_inactive() {
		// 		self.inputs = Inputs::MousePointer(MousePointer::new().unwrap());
		// 	}
		// 	// if Input::key(Key::F7).is_just_inactive() {
		// 	// 	self.inputs = Inputs::Controllers((
		// 	// 		SkController::new(Handed::Left).unwrap(),
		// 	// 		SkController::new(Handed::Right).unwrap(),
		// 	// 	));
		// 	// }
		// 	if Input::key(Key::F8).is_just_inactive() {
		// 		self.inputs = Inputs::Hands {
		// 			left: SkHand::new(&self.connection, Handed::Left).unwrap(),
		// 			right: SkHand::new(&self.connection, Handed::Right).unwrap(),
		// 		};
		// 	}
		// }

		match &mut self.inputs {
			Some(Inputs::XR {
				controller_left,
				controller_right,
				hand_left,
				hand_right,
				eye_pointer,
			}) => {
				// controller_left.update(token);
				// controller_right.update(token);
				// hand_left.update(sk, token);
				// hand_right.update(sk, token);
				if let Some(eye_pointer) = eye_pointer {
					eye_pointer.update();
				}
			}
			Some(Inputs::MousePointer(mouse_pointer)) => mouse_pointer.update(),
			// Inputs::Controllers((left, right)) => {
			// 	left.update(token);
			// 	right.update(token);
			// }
			Some(Inputs::Hands { left, right }) => {
				// left.update(sk, token);
				// right.update(sk, token);
			}
			None => {}
		}
	}
	pub fn set_inputs(&mut self, inputs: Inputs) {
		self.inputs = Some(inputs);
	}
	pub fn unset_inputs(&mut self, inputs: Inputs) {
		self.inputs = None;
	}
}

pub struct ObjectHandle<I: Interface>(Connection, OwnedObjectPath, PhantomData<I>);
impl<I: Interface> Drop for ObjectHandle<I> {
	fn drop(&mut self) {
		let connection = self.0.clone();
		let object_path = self.1.clone();
		TOKIO.spawn(async move {
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

		TOKIO.spawn({
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

		TOKIO.spawn({
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

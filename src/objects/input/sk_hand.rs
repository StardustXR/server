use crate::bevy_plugin::DbusConnection;
use crate::core::client::INTERNAL_CLIENT;
use crate::nodes::fields::{Field, FieldTrait};
use crate::nodes::input::{InputDataType, InputHandler, INPUT_HANDLER_REGISTRY};
use crate::nodes::OwnedNode;
use crate::nodes::{
	input::{Hand, InputMethod, Joint},
	spatial::Spatial,
	Node,
};
use crate::objects::{ObjectHandle, SpatialRef};
use crate::DefaultMaterial;
use bevy::asset::{AssetServer, Assets, Handle};
use bevy::prelude::{Commands, Component, Entity, Query, Res, ResMut};
use bevy_mod_openxr::helper_traits::{ToQuat, ToVec3};
use bevy_mod_openxr::resources::OxrFrameState;
use bevy_mod_openxr::session::OxrSession;
use bevy_mod_openxr::spaces::OxrSpaceLocationFlags;
use bevy_mod_xr::hands::{HandBone, HandSide};
use bevy_mod_xr::spaces::XrPrimaryReferenceSpace;
use color_eyre::eyre::Result;
use glam::{Mat4, Quat, Vec3};
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use stardust_xr::values::Datamap;
use std::sync::Arc;
use tracing::error;
use zbus::Connection;

fn update_joint(joint: &mut Joint, oxr_joint: openxr::HandJointLocation) {
	let flags = OxrSpaceLocationFlags(oxr_joint.location_flags);
	if flags.pos_valid() && flags.rot_valid() {
		*joint = convert_joint(oxr_joint);
	}
}

fn update_hands(
	mut mats: ResMut<Assets<DefaultMaterial>>,
	mut query: Query<&mut SkHand>,
	time: Res<OxrFrameState>,
	base_space: Res<XrPrimaryReferenceSpace>,
	session: ResMut<OxrSession>,
) {
	for mut hand in &mut query {
		let joints = session
			.locate_hand_joints(&hand.hand_tracker, &base_space, time.predicted_display_time)
			.unwrap();
		if let InputDataType::Hand(hand_input) = &mut *hand.input.data.lock() {
			let input_node = hand.input.spatial.node().unwrap();
			input_node.set_enabled(joints.is_some());
			if let Some(joints) = joints.as_ref() {
				update_joint(
					&mut hand_input.thumb.tip,
					joints[HandBone::ThumbTip as usize],
				);
				update_joint(
					&mut hand_input.thumb.distal,
					joints[HandBone::ThumbDistal as usize],
				);
				update_joint(
					&mut hand_input.thumb.proximal,
					joints[HandBone::ThumbProximal as usize],
				);
				update_joint(
					&mut hand_input.thumb.metacarpal,
					joints[HandBone::ThumbMetacarpal as usize],
				);

				for (finger, finger_index) in [
					(&mut hand_input.index, 6),
					(&mut hand_input.middle, 11),
					(&mut hand_input.ring, 16),
					(&mut hand_input.little, 21),
				] {
					update_joint(&mut finger.tip, joints[finger_index + 4]);
					update_joint(&mut finger.distal, joints[finger_index + 3]);
					update_joint(&mut finger.intermediate, joints[finger_index + 2]);
					update_joint(&mut finger.proximal, joints[finger_index + 1]);
					update_joint(&mut finger.metacarpal, joints[finger_index + 0]);
					// Why?
					finger.tip.radius = 0.0;
				}
				update_joint(&mut hand_input.palm, joints[HandBone::Palm as usize]);
				hand.palm_spatial
					.set_local_transform(Mat4::from_rotation_translation(
						hand_input.palm.rotation.into(),
						hand_input.palm.position.into(),
					));
				update_joint(&mut hand_input.wrist, joints[HandBone::Wrist as usize]);

				hand_input.elbow = None;
			}
		}
		if let Some(joints) = joints.as_ref() {
			hand.datamap.pinch_strength = pinch_activation(joints);
			hand.datamap.grab_strength = grip_activation(joints);
			*hand.input.datamap.lock() = Datamap::from_typed(&hand.datamap).unwrap();
		}
		// remove the capture when it's removed from captures list
		if let Some(capture) = &hand.capture {
			if !hand
				.input
				.capture_requests
				.get_valid_contents()
				.contains(capture)
			{
				hand.capture.take();
			}
		}
		// add the capture that's the closest if we don't have one
		if hand.capture.is_none() {
			hand.capture = hand
				.input
				.capture_requests
				.get_valid_contents()
				.into_iter()
				.map(|handler| (handler.clone(), hand.compare_distance(&handler.field).abs()))
				.reduce(|(handlers_a, distance_a), (handlers_b, distance_b)| {
					if distance_a < distance_b {
						(handlers_a, distance_a)
					} else {
						(handlers_b, distance_b)
					}
				})
				.map(|(rx, _)| rx);
		}

		// make sure that if something is captured only send input to it
		hand.input.captures.clear();
		if let Some(capture) = &hand.capture {
			hand.input.set_handler_order([capture].into_iter());
			hand.input.captures.add_raw(capture);
			return;
		}

		// send input to all the input handlers that are the closest to the ray as possible
		hand.input.set_handler_order(
			INPUT_HANDLER_REGISTRY
				.get_valid_contents()
				.into_iter()
				// filter out all the disabled handlers
				.filter(|handler| {
					let Some(node) = handler.spatial.node() else {
						return false;
					};
					node.enabled()
				})
				// filter out all the fields with disabled handlers
				.filter(|handler| {
					let Some(node) = handler.field.spatial.node() else {
						return false;
					};
					node.enabled()
				})
				// get the unsigned distance to the handler's field (unsigned so giant fields won't always eat input)
				.map(|handler| {
					(
						vec![handler.clone()],
						hand.compare_distance(&handler.field).abs(),
					)
				})
				// .inspect(|(_, result)| {
				// 	dbg!(result);
				// })
				// now collect all handlers that are same distance if they're the closest
				.reduce(|(mut handlers_a, distance_a), (handlers_b, distance_b)| {
					if (distance_a - distance_b).abs() < 0.001 {
						// distance is basically the same (within 1mm)
						handlers_a.extend(handlers_b);
						(handlers_a, distance_a)
					} else if distance_a < distance_b {
						(handlers_a, distance_a)
					} else {
						(handlers_b, distance_b)
					}
				})
				.map(|(rx, _)| rx)
				.unwrap_or_default()
				.iter(),
		);
	}
}

const PINCH_MAX: f32 = 0.11;
const PINCH_ACTIVACTION_DISTANCE: f32 = 0.01;
// TODO: handle invalid data
// based on https://github.com/StereoKit/StereoKit/blob/ca2be7d45f4f4388e8df7542e9a0313bcc45946e/StereoKitC/hands/input_hand.cpp#L375-L394
fn pinch_activation(joints: &[openxr::HandJointLocation; openxr::HAND_JOINT_COUNT]) -> f32 {
	let combined_radius =
		joints[HandBone::ThumbTip as usize].radius + joints[HandBone::IndexTip as usize].radius;
	let pinch_dist = joints[HandBone::ThumbTip as usize]
		.pose
		.position
		.to_vec3()
		.distance(joints[HandBone::IndexTip as usize].pose.position.to_vec3())
		- combined_radius;
	(1.0 - ((pinch_dist - PINCH_ACTIVACTION_DISTANCE) / (PINCH_MAX - PINCH_ACTIVACTION_DISTANCE)))
		.clamp(0.0, 1.0)
}

const GRIP_MAX: f32 = 0.11;
const GRIP_ACTIVACTION_DISTANCE: f32 = 0.01;
// TODO: handle invalid data
// based on https://github.com/StereoKit/StereoKit/blob/ca2be7d45f4f4388e8df7542e9a0313bcc45946e/StereoKitC/hands/input_hand.cpp#L375-L394
fn grip_activation(joints: &[openxr::HandJointLocation; openxr::HAND_JOINT_COUNT]) -> f32 {
	let combined_radius = joints[HandBone::RingTip as usize].radius
		+ joints[HandBone::RingMetacarpal as usize].radius;
	let grip_dist = joints[HandBone::RingTip as usize]
		.pose
		.position
		.to_vec3()
		.distance(
			joints[HandBone::RingMetacarpal as usize]
				.pose
				.position
				.to_vec3(),
		) - combined_radius;
	(1.0 - ((grip_dist - GRIP_ACTIVACTION_DISTANCE) / (GRIP_MAX - GRIP_ACTIVACTION_DISTANCE)))
		.clamp(0.0, 1.0)
}

fn create_hands(connection: Res<DbusConnection>, session: Res<OxrSession>, mut cmds: Commands) {
	for handed in [HandSide::Left, HandSide::Right] {
		let hand = (|| -> color_eyre::Result<_> {
			let side = match handed {
				HandSide::Left => "left",
				HandSide::Right => "right",
			};
			let (palm_spatial, palm_object) = SpatialRef::create(
				&connection,
				&("/org/stardustxr/Hand/".to_string() + side + "/palm"),
			);
			let _node = Node::generate(&INTERNAL_CLIENT, false).add_to_scenegraph_owned()?;
			Spatial::add_to(&_node.0, None, Mat4::IDENTITY, false);
			let hand = InputDataType::Hand(Hand {
				right: matches!(handed, HandSide::Right),
				..Default::default()
			});
			let datamap = Datamap::from_typed(HandDatamap::default())?;
			let input = InputMethod::add_to(&_node.0, hand, datamap)?;

			let tracker = session.create_hand_tracker(match handed {
				HandSide::Left => openxr::Hand::LEFT,
				HandSide::Right => openxr::Hand::RIGHT,
			})?;

			Ok(SkHand {
				_node,
				palm_spatial,
				palm_object,
				handed,
				input,
				capture: None,
				datamap: Default::default(),
				material: OnceCell::new(),
				vis_entity: OnceCell::new(),
				hand_tracker: tracker,
			})
		})();
		let hand = match hand {
			Ok(v) => v,
			Err(err) => {
				error!("error while creating hand: {err}");
				continue;
			}
		};
		cmds.spawn(hand);
	}
}

fn convert_joint(joint: openxr::HandJointLocation) -> Joint {
	Joint {
		position: joint.pose.position.to_vec3().into(),
		rotation: joint.pose.orientation.to_quat().into(),
		radius: joint.radius,
		distance: 0.0,
	}
}

#[derive(Default, Deserialize, Serialize)]
struct HandDatamap {
	pinch_strength: f32,
	grab_strength: f32,
}

#[derive(Component)]
pub struct SkHand {
	_node: OwnedNode,
	palm_spatial: Arc<Spatial>,
	palm_object: ObjectHandle<SpatialRef>,
	handed: HandSide,
	input: Arc<InputMethod>,
	capture: Option<Arc<InputHandler>>,
	datamap: HandDatamap,
	material: OnceCell<Handle<DefaultMaterial>>,
	vis_entity: OnceCell<Entity>,
	hand_tracker: openxr::HandTracker,
}
impl SkHand {
	fn compare_distance(&self, field: &Field) -> f32 {
		let InputDataType::Hand(hand) = &*self.input.data.lock() else {
			return INFINITY;
		};
		let spatial = &self.input.spatial;
		let thumb_tip_distance = field.distance(spatial, hand.thumb.tip.position.into());
		let index_tip_distance = field.distance(spatial, hand.index.tip.position.into());
		let middle_tip_distance = field.distance(spatial, hand.middle.tip.position.into());
		let ring_tip_distance = field.distance(spatial, hand.ring.tip.position.into());

		(thumb_tip_distance * 0.3)
			+ (index_tip_distance * 0.4)
			+ (middle_tip_distance * 0.15)
			+ (ring_tip_distance * 0.15)
	}
}

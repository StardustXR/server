use crate::bevy_plugin::{DbusConnection, InputUpdate};
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
use bevy::app::{Plugin, PostUpdate};
use bevy::asset::{AssetServer, Assets, Handle};
use bevy::prelude::{
	Commands, Component, Entity, Gizmos, IntoSystemConfigs as _, Query, Res, ResMut,
};
use bevy::utils::default;
use bevy_mod_openxr::helper_traits::{ToQuat, ToVec3};
use bevy_mod_openxr::openxr_session_running;
use bevy_mod_openxr::resources::{OxrFrameState, Pipelined};
use bevy_mod_openxr::session::OxrSession;
use bevy_mod_openxr::spaces::OxrSpaceLocationFlags;
use bevy_mod_xr::hands::{HandBone, HandSide};
use bevy_mod_xr::session::XrSessionCreated;
use bevy_mod_xr::spaces::XrPrimaryReferenceSpace;
use color_eyre::eyre::Result;
use glam::{Mat4, Quat, Vec3};
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use stardust_xr::values::Datamap;
use std::sync::Arc;
use tracing::error;
use zbus::Connection;

use super::{get_sorted_handlers, CaptureManager};

fn update_joint(joint: &mut Joint, oxr_joint: openxr::HandJointLocation) {
	let flags = OxrSpaceLocationFlags(oxr_joint.location_flags);
	if flags.pos_valid() && flags.rot_valid() {
		*joint = convert_joint(oxr_joint);
	}
}

pub struct StardustHandPlugin;
impl Plugin for StardustHandPlugin {
	fn build(&self, app: &mut bevy::prelude::App) {
		app.add_systems(XrSessionCreated, create_hands);
		app.add_systems(
			InputUpdate,
			(
				update_hands.run_if(openxr_session_running),
				draw_hand_gizmos,
			)
				.chain(),
		);
	}
}

fn draw_hand_gizmos(mut gizmos: Gizmos, query: Query<&SkHand>) {
	for hand in query.iter() {
		gizmos.axes(hand.palm_spatial.global_transform(), 0.05);
	}
}

fn update_hands(
	mut mats: ResMut<Assets<DefaultMaterial>>,
	mut query: Query<&mut SkHand>,
	time: Res<OxrFrameState>,
	base_space: Res<XrPrimaryReferenceSpace>,
	session: ResMut<OxrSession>,
	pipelined: Option<Res<Pipelined>>,
) {
	let time = if pipelined.is_some() {
		openxr::Time::from_nanos(
			time.predicted_display_time.as_nanos() + time.predicted_display_period.as_nanos(),
		)
	} else {
		time.predicted_display_time
	};
	for mut hand in &mut query {
		let joints = session
			.locate_hand_joints(&hand.hand_tracker, &base_space, time)
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
					update_joint(&mut finger.metacarpal, joints[finger_index]);
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
		let distance_calculator = |space: &Arc<Spatial>, data: &InputDataType, field: &Field| {
			let InputDataType::Hand(hand) = data else {
				return None;
			};
			let thumb_tip_distance = field.distance(space, hand.thumb.tip.position.into());
			let index_tip_distance = field.distance(space, hand.index.tip.position.into());
			let middle_tip_distance = field.distance(space, hand.middle.tip.position.into());
			let ring_tip_distance = field.distance(space, hand.ring.tip.position.into());

			Some(
				(thumb_tip_distance * 0.3)
					+ (index_tip_distance * 0.4)
					+ (middle_tip_distance * 0.15)
					+ (ring_tip_distance * 0.15),
			)
		};

		let input = hand.input.clone();
		hand.capture_manager.update_capture(&input);
		hand.capture_manager
			.set_new_capture(&input, distance_calculator);
		hand.capture_manager.apply_capture(&input);

		if hand.capture_manager.capture.is_some() {
			return;
		}

		let sorted_handlers = get_sorted_handlers(&hand.input, distance_calculator);
		hand.input.set_handler_order(sorted_handlers.iter());
	}
}

const PINCH_MAX: f32 = 0.11;
const PINCH_ACTIVACTION_DISTANCE: f32 = 0.01;
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
				capture_manager: default(),
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
	datamap: HandDatamap,
	material: OnceCell<Handle<DefaultMaterial>>,
	vis_entity: OnceCell<Entity>,
	hand_tracker: openxr::HandTracker,
	capture_manager: CaptureManager,
}
impl SkHand {
	fn compare_distance(&self, field: &Field) -> f32 {
		let InputDataType::Hand(hand) = &*self.input.data.lock() else {
			return f32::INFINITY;
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

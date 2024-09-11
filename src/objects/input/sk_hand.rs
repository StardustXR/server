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
use color_eyre::eyre::Result;
use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};
use stardust_xr::values::Datamap;
use std::sync::Arc;
use stereokit_rust::sk::{DisplayMode, MainThreadToken, Sk};
use stereokit_rust::system::{HandJoint, HandSource, Handed, Input, LinePoint, Lines};
use stereokit_rust::util::Color128;
use zbus::Connection;

use super::{get_sorted_handlers, CaptureManager};

fn convert_joint(joint: HandJoint) -> Joint {
	Joint {
		position: Vec3::from(joint.position).into(),
		rotation: Quat::from(joint.orientation).into(),
		radius: joint.radius,
		distance: 0.0,
	}
}

#[derive(Default, Deserialize, Serialize)]
struct HandDatamap {
	pinch_strength: f32,
	grab_strength: f32,
}

pub struct SkHand {
	_node: OwnedNode,
	palm_spatial: Arc<Spatial>,
	palm_object: ObjectHandle<SpatialRef>,
	handed: Handed,
	input: Arc<InputMethod>,
	capture_manager: CaptureManager,
	datamap: HandDatamap,
}
impl SkHand {
	pub fn new(connection: &Connection, handed: Handed) -> Result<Self> {
		let (palm_spatial, palm_object) = SpatialRef::create(
			connection,
			&("/org/stardustxr/Hand/".to_string()
				+ match handed {
					Handed::Left => "left",
					_ => "right",
				} + "/palm"),
		);
		let _node = Node::generate(&INTERNAL_CLIENT, false).add_to_scenegraph_owned()?;
		Spatial::add_to(&_node.0, None, Mat4::IDENTITY, false);
		let hand = InputDataType::Hand(Hand {
			right: handed == Handed::Right,
			..Default::default()
		});
		let datamap = Datamap::from_typed(HandDatamap::default())?;
		let input = InputMethod::add_to(&_node.0, hand, datamap)?;

		Input::hand_visible(handed, false);
		Ok(SkHand {
			_node,
			palm_spatial,
			palm_object,
			handed,
			input,
			capture_manager: CaptureManager::default(),
			datamap: Default::default(),
		})
	}
	pub fn update(&mut self, sk: &Sk, token: &MainThreadToken) {
		let sk_hand = Input::hand(self.handed);
		let real_hand = Input::hand_source(self.handed) as u32 == HandSource::Articulated as u32;
		if let InputDataType::Hand(hand) = &mut *self.input.data.lock() {
			let input_node = self.input.spatial.node().unwrap();
			input_node.set_enabled(
				(real_hand || sk.get_active_display_mode() == DisplayMode::Flatscreen)
					&& sk_hand.tracked.is_active(),
			);
			if input_node.enabled() {
				hand.thumb.tip = convert_joint(sk_hand.fingers[0][4]);
				hand.thumb.distal = convert_joint(sk_hand.fingers[0][3]);
				hand.thumb.proximal = convert_joint(sk_hand.fingers[0][2]);
				hand.thumb.metacarpal = convert_joint(sk_hand.fingers[0][1]);

				for (finger, mut sk_finger) in [
					(&mut hand.index, sk_hand.fingers[1]),
					(&mut hand.middle, sk_hand.fingers[2]),
					(&mut hand.ring, sk_hand.fingers[3]),
					(&mut hand.little, sk_hand.fingers[4]),
				] {
					sk_finger[4].radius = 0.0;
					finger.tip = convert_joint(sk_finger[4]);
					finger.distal = convert_joint(sk_finger[3]);
					finger.intermediate = convert_joint(sk_finger[2]);
					finger.proximal = convert_joint(sk_finger[1]);
					finger.metacarpal = convert_joint(sk_finger[0]);
				}

				hand.palm.position = Vec3::from(sk_hand.palm.position).into();
				hand.palm.rotation = Quat::from(sk_hand.palm.orientation).into();
				hand.palm.radius =
					(sk_hand.fingers[2][0].radius + sk_hand.fingers[2][1].radius) * 0.5;

				self.palm_spatial
					.set_local_transform(Mat4::from_rotation_translation(
						hand.palm.rotation.into(),
						hand.palm.position.into(),
					));

				hand.wrist.position = Vec3::from(sk_hand.wrist.position).into();
				hand.wrist.rotation = Quat::from(sk_hand.wrist.orientation).into();
				hand.wrist.radius =
					(sk_hand.fingers[0][0].radius + sk_hand.fingers[4][0].radius) * 0.5;

				hand.elbow = None;

				self.draw(
					token,
					if self.capture_manager.capture.is_none() {
						Color128::new_rgb(1.0, 1.0, 1.0)
					} else {
						Color128::new_rgb(0.0, 1.0, 0.75)
					},
					hand,
				);
			}
		}
		self.datamap.pinch_strength = sk_hand.pinch_activation;
		self.datamap.grab_strength = sk_hand.grip_activation;
		*self.input.datamap.lock() = Datamap::from_typed(&self.datamap).unwrap();

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

		self.capture_manager.update_capture(&self.input);
		self.capture_manager
			.set_new_capture(&self.input, distance_calculator);
		self.capture_manager.apply_capture(&self.input);

		if self.capture_manager.capture.is_some() {
			return;
		}

		let sorted_handlers = get_sorted_handlers(&self.input, distance_calculator);
		self.input.set_handler_order(sorted_handlers.iter());
	}

	fn draw(&self, token: &MainThreadToken, color: Color128, hand: &Hand) {
		// thumb
		Lines::add_list(
			token,
			&[
				joint_to_line_point(&hand.thumb.tip, color),
				joint_to_line_point(&hand.thumb.distal, color),
				joint_to_line_point(&hand.thumb.proximal, color),
				joint_to_line_point(&hand.thumb.metacarpal, color),
			],
		);
		// index
		Lines::add_list(
			token,
			&[
				joint_to_line_point(&hand.index.tip, color),
				joint_to_line_point(&hand.index.distal, color),
				joint_to_line_point(&hand.index.intermediate, color),
				joint_to_line_point(&hand.index.proximal, color),
				joint_to_line_point(&hand.index.metacarpal, color),
			],
		);
		// middle
		Lines::add_list(
			token,
			&[
				joint_to_line_point(&hand.middle.tip, color),
				joint_to_line_point(&hand.middle.distal, color),
				joint_to_line_point(&hand.middle.intermediate, color),
				joint_to_line_point(&hand.middle.proximal, color),
				joint_to_line_point(&hand.middle.metacarpal, color),
			],
		);
		// ring
		Lines::add_list(
			token,
			&[
				joint_to_line_point(&hand.ring.tip, color),
				joint_to_line_point(&hand.ring.distal, color),
				joint_to_line_point(&hand.ring.intermediate, color),
				joint_to_line_point(&hand.ring.proximal, color),
				joint_to_line_point(&hand.ring.metacarpal, color),
			],
		);
		// little
		Lines::add_list(
			token,
			&[
				joint_to_line_point(&hand.little.tip, color),
				joint_to_line_point(&hand.little.distal, color),
				joint_to_line_point(&hand.little.intermediate, color),
				joint_to_line_point(&hand.little.proximal, color),
				joint_to_line_point(&hand.little.metacarpal, color),
			],
		);

		// palm
		Lines::add_list(
			token,
			&[
				joint_to_line_point(&hand.wrist, color),
				joint_to_line_point(&hand.thumb.metacarpal, color),
				joint_to_line_point(&hand.index.metacarpal, color),
				joint_to_line_point(&hand.middle.metacarpal, color),
				joint_to_line_point(&hand.ring.metacarpal, color),
				joint_to_line_point(&hand.little.metacarpal, color),
				joint_to_line_point(&hand.wrist, color),
			],
		);
	}
}

fn joint_to_line_point(joint: &Joint, color: Color128) -> LinePoint {
	LinePoint {
		pt: Vec3::from(joint.position).into(),
		thickness: joint.radius * 2.0,
		color: color.into(),
	}
}

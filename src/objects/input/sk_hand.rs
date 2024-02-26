use crate::core::client::INTERNAL_CLIENT;
use crate::nodes::input::InputDataType;
use crate::nodes::{
	input::{Hand, InputMethod, Joint},
	spatial::Spatial,
	Node,
};
use color_eyre::eyre::Result;
use glam::Mat4;
use nanoid::nanoid;
use serde::{Deserialize, Serialize};
use stardust_xr::values::Datamap;
use std::sync::Arc;
use stereokit::{ButtonState, HandJoint, Handed, StereoKitMultiThread};

fn convert_joint(joint: HandJoint) -> Joint {
	Joint {
		position: joint.position.into(),
		rotation: joint.orientation.into(),
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
	_node: Arc<Node>,
	input: Arc<InputMethod>,
	handed: Handed,
	datamap: HandDatamap,
}
impl SkHand {
	pub fn new(handed: Handed) -> Result<Self> {
		let _node = Node::create_parent_name(&INTERNAL_CLIENT, "", &nanoid!(), false)
			.add_to_scenegraph()?;
		Spatial::add_to(&_node, None, Mat4::IDENTITY, false);
		let hand = InputDataType::Hand(Hand {
			right: handed == Handed::Right,
			..Default::default()
		});
		let datamap = Datamap::from_typed(HandDatamap::default())?;
		let input = InputMethod::add_to(&_node, hand, datamap)?;
		Ok(SkHand {
			_node,
			input,
			handed,
			datamap: Default::default(),
		})
	}
	pub fn update(&mut self, controller_enabled: bool, sk: &impl StereoKitMultiThread) {
		let sk_hand = sk.input_hand(self.handed);
		if let InputDataType::Hand(hand) = &mut *self.input.data.lock() {
			let controller_active = controller_enabled
				&& sk
					.input_controller(self.handed)
					.tracked
					.contains(ButtonState::ACTIVE);
			*self.input.enabled.lock() =
				!controller_active && sk_hand.tracked_state.contains(ButtonState::ACTIVE);
			sk.input_hand_visible(self.handed, *self.input.enabled.lock());
			if *self.input.enabled.lock() {
				hand.thumb.tip = convert_joint(sk_hand.fingers[0][4]);
				hand.thumb.distal = convert_joint(sk_hand.fingers[0][3]);
				hand.thumb.proximal = convert_joint(sk_hand.fingers[0][2]);
				hand.thumb.metacarpal = convert_joint(sk_hand.fingers[0][1]);

				for (finger, sk_finger) in [
					(&mut hand.index, sk_hand.fingers[1]),
					(&mut hand.middle, sk_hand.fingers[2]),
					(&mut hand.ring, sk_hand.fingers[3]),
					(&mut hand.little, sk_hand.fingers[4]),
				] {
					finger.tip = convert_joint(sk_finger[4]);
					finger.distal = convert_joint(sk_finger[3]);
					finger.intermediate = convert_joint(sk_finger[2]);
					finger.proximal = convert_joint(sk_finger[1]);
					finger.metacarpal = convert_joint(sk_finger[0]);
				}

				hand.palm.position = sk_hand.palm.position.into();
				hand.palm.rotation = sk_hand.palm.orientation.into();
				hand.palm.radius =
					(sk_hand.fingers[2][0].radius + sk_hand.fingers[2][1].radius) * 0.5;

				hand.wrist.position = sk_hand.wrist.position.into();
				hand.wrist.rotation = sk_hand.wrist.orientation.into();
				hand.wrist.radius =
					(sk_hand.fingers[0][0].radius + sk_hand.fingers[4][0].radius) * 0.5;

				hand.elbow = None;
			}
		}
		self.datamap.pinch_strength = sk_hand.pinch_activation;
		self.datamap.grab_strength = sk_hand.grip_activation;
		*self.input.datamap.lock() = Datamap::from_typed(&self.datamap).unwrap();
	}
}

use crate::nodes::{
	input::{InputMethod, InputType},
	spatial::Spatial,
};
use glam::Mat4;
use libstardustxr::schemas::{common::JointT, input_hand::HandT};
use std::sync::{Arc, Weak};
use stereokit::{
	input::{Handed, Joint as SkJoint},
	StereoKit,
};

fn convert_joint(joint: SkJoint) -> JointT {
	JointT {
		position: joint.position.into(),
		rotation: joint.orientation.into(),
		radius: joint.radius,
	}
}

pub struct SkHand {
	hand: Arc<InputMethod>,
	handed: Handed,
}
impl SkHand {
	pub fn new(handed: Handed) -> Self {
		let mut sk_hand = HandT::default();
		sk_hand.right = handed == Handed::Right;
		SkHand {
			hand: InputMethod::new(
				Spatial::new(Weak::new(), None, Mat4::IDENTITY),
				InputType::Hand(Box::new(sk_hand)),
			),
			handed,
		}
	}
	pub fn update(&mut self, sk: &StereoKit) {
		if let InputType::Hand(hand) = &mut *self.hand.specialization.lock() {
			let sk_hand = *sk.input_hand(self.handed);
			// *self.hand.enabled.lock() = sk_hand.tracked_state.is_active();
			// if sk_hand.tracked_state.is_active() {
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
			hand.palm.radius = (sk_hand.fingers[2][0].radius + sk_hand.fingers[2][1].radius) * 0.5;

			hand.wrist.position = sk_hand.wrist.position.into();
			hand.wrist.rotation = sk_hand.wrist.orientation.into();
			hand.wrist.radius = (sk_hand.fingers[0][0].radius + sk_hand.fingers[4][0].radius) * 0.5;

			hand.elbow = None;
			// }
		}
	}
}

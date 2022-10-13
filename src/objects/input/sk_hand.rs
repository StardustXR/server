use crate::nodes::{
	input::{hand::Hand, InputMethod, InputType},
	spatial::Spatial,
};
use glam::Mat4;
use stardust_xr::schemas::{common::JointT, input_hand::HandT};
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
				InputType::Hand(Box::new(Hand {
					base: sk_hand,
					pinch_strength: 0.0,
					grab_strength: 0.0,
				})),
			),
			handed,
		}
	}
	pub fn update(&mut self, sk: &StereoKit) {
		if let InputType::Hand(hand) = &mut *self.hand.specialization.lock() {
			let sk_hand = *sk.input_hand(self.handed);
			let controller = sk.input_controller(self.handed);
			*self.hand.enabled.lock() =
				controller.tracked.is_inactive() && sk_hand.tracked_state.is_active();
			if *self.hand.enabled.lock() {
				hand.base.thumb.tip = convert_joint(sk_hand.fingers[0][4]);
				hand.base.thumb.distal = convert_joint(sk_hand.fingers[0][3]);
				hand.base.thumb.proximal = convert_joint(sk_hand.fingers[0][2]);
				hand.base.thumb.metacarpal = convert_joint(sk_hand.fingers[0][1]);

				for (finger, sk_finger) in [
					(&mut hand.base.index, sk_hand.fingers[1]),
					(&mut hand.base.middle, sk_hand.fingers[2]),
					(&mut hand.base.ring, sk_hand.fingers[3]),
					(&mut hand.base.little, sk_hand.fingers[4]),
				] {
					finger.tip = convert_joint(sk_finger[4]);
					finger.distal = convert_joint(sk_finger[3]);
					finger.intermediate = convert_joint(sk_finger[2]);
					finger.proximal = convert_joint(sk_finger[1]);
					finger.metacarpal = convert_joint(sk_finger[0]);
				}

				hand.base.palm.position = sk_hand.palm.position.into();
				hand.base.palm.rotation = sk_hand.palm.orientation.into();
				hand.base.palm.radius =
					(sk_hand.fingers[2][0].radius + sk_hand.fingers[2][1].radius) * 0.5;

				hand.base.wrist.position = sk_hand.wrist.position.into();
				hand.base.wrist.rotation = sk_hand.wrist.orientation.into();
				hand.base.wrist.radius =
					(sk_hand.fingers[0][0].radius + sk_hand.fingers[4][0].radius) * 0.5;

				hand.base.elbow = None;

				//hand.pinch_strength = sk_hand.pinch_activation;
				//hand.grab_strength = sk_hand.grip_activation;
				hand.pinch_strength = if sk_hand.pinch_state.is_active() {
					1.0
				} else {
					0.0
				};
				hand.grab_strength = if sk_hand.grip_state.is_active() {
					1.0
				} else {
					0.0
				};
			}
		}
	}
}

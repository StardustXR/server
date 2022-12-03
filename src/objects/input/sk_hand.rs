use crate::nodes::{
	input::{hand::Hand, InputMethod, InputType},
	spatial::Spatial,
};
use glam::Mat4;
use stardust_xr::schemas::flat::{Datamap, Hand as FlatHand, Joint};
use std::sync::{Arc, Weak};
use stereokit::{
	input::{ButtonState, Handed, Joint as SkJoint},
	StereoKit,
};

fn convert_joint(joint: SkJoint) -> Joint {
	Joint {
		position: joint.position,
		rotation: joint.orientation,
		radius: joint.radius,
	}
}

pub struct SkHand {
	hand: Arc<InputMethod>,
	handed: Handed,
}
impl SkHand {
	pub fn new(handed: Handed) -> Self {
		SkHand {
			hand: InputMethod::new(
				Spatial::new(Weak::new(), None, Mat4::IDENTITY),
				InputType::Hand(Box::new(Hand {
					base: FlatHand {
						right: handed == Handed::Right,
						..Default::default()
					},
				})),
			),
			handed,
		}
	}
	pub fn update(&mut self, sk: &StereoKit) {
		let sk_hand = sk.input_hand(self.handed);
		if let InputType::Hand(hand) = &mut *self.hand.specialization.lock() {
			let controller = sk.input_controller(self.handed);
			*self.hand.enabled.lock() = controller.tracked.contains(ButtonState::Inactive)
				&& sk_hand.tracked_state.contains(ButtonState::Active);
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

				hand.base.palm.position = sk_hand.palm.position;
				hand.base.palm.rotation = sk_hand.palm.orientation;
				hand.base.palm.radius =
					(sk_hand.fingers[2][0].radius + sk_hand.fingers[2][1].radius) * 0.5;

				hand.base.wrist.position = sk_hand.wrist.position;
				hand.base.wrist.rotation = sk_hand.wrist.orientation;
				hand.base.wrist.radius =
					(sk_hand.fingers[0][0].radius + sk_hand.fingers[4][0].radius) * 0.5;

				hand.base.elbow = None;
			}
		}
		let mut fbb = flexbuffers::Builder::default();
		let mut map = fbb.start_map();
		map.push("grab_strength", sk_hand.grip_activation);
		map.push("pinch_strength", sk_hand.pinch_activation);
		map.end_map();
		*self.hand.datamap.lock() = Datamap::new(fbb.take_buffer()).ok();
	}
}

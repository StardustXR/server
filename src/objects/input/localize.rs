use crate::nodes::{
	fields::{Field, Ray},
	spatial::Spatial,
};
use glam::{Mat4, Quat, Vec3, Vec3A};
use stardust_xr_protocol::{
	suis::{Finger, Hand, InputDataType, Joint, Pointer, SpatialData, Thumb, Tip},
	types::Posef,
};
use std::sync::Arc;

/// Move input data from the space it was made in into an input handler's space, filling in
/// every distance the handler expects along the way.
pub fn localize(
	from: &Arc<Spatial>,
	handler_spatial: &Spatial,
	handler_field: &Field,
	input: &InputDataType,
) -> Option<SpatialData> {
	let to_handler = Spatial::space_to_space_matrix(Some(from), Some(handler_spatial));
	let ctx = Localize {
		from,
		handler_field,
		to_handler,
		rotation: to_handler.to_scale_rotation_translation().1,
	};

	Some(match input {
		InputDataType::Pointer { data } => ctx.pointer(data)?,
		InputDataType::Hand { data } => ctx.hand(data),
		InputDataType::Tip { data } => ctx.tip(data),
	})
}

struct Localize<'a> {
	from: &'a Arc<Spatial>,
	handler_field: &'a Field,
	to_handler: Mat4,
	rotation: Quat,
}
impl Localize<'_> {
	fn pose(&self, pose: Posef) -> Posef {
		Posef {
			position: self
				.to_handler
				.transform_point3a(pose.position.into())
				.into(),
			orientation: (self.rotation * Quat::from(pose.orientation)).into(),
		}
	}

	/// distance is sampled at the untransformed position in the method's own space, which is
	/// the same point in the world and doesn't lean on the transform being right
	fn distance(&self, position: impl Into<Vec3A>) -> f32 {
		self.handler_field
			.sample(self.from, position.into())
			.distance
	}

	fn joint(&self, joint: &Joint) -> Joint {
		Joint {
			pose: self.pose(joint.pose),
			radius: joint.radius,
			distance: self.distance(joint.pose.position),
		}
	}

	fn finger(&self, finger: &Finger) -> Finger {
		Finger {
			tip: self.joint(&finger.tip),
			distal: self.joint(&finger.distal),
			intermediate: self.joint(&finger.intermediate),
			proximal: self.joint(&finger.proximal),
			metacarpal: self.joint(&finger.metacarpal),
		}
	}

	fn thumb(&self, thumb: &Thumb) -> Thumb {
		Thumb {
			tip: self.joint(&thumb.tip),
			distal: self.joint(&thumb.distal),
			proximal: self.joint(&thumb.proximal),
			metacarpal: self.joint(&thumb.metacarpal),
		}
	}

	fn localized_hand(&self, hand: &Hand) -> Hand {
		Hand {
			chirality: hand.chirality,
			thumb: self.thumb(&hand.thumb),
			index: self.finger(&hand.index),
			middle: self.finger(&hand.middle),
			ring: self.finger(&hand.ring),
			little: self.finger(&hand.little),
			palm: self.joint(&hand.palm),
			wrist: self.joint(&hand.wrist),
			elbow: hand.elbow.as_ref().map(|j| self.joint(j)),
		}
	}

	fn hand(&self, hand: &Hand) -> SpatialData {
		let hand = self.localized_hand(hand);
		SpatialData {
			distance: hand_distance(&hand),
			input: InputDataType::Hand { data: hand },
		}
	}

	fn tip(&self, tip: &Tip) -> SpatialData {
		SpatialData {
			distance: self.distance(tip.pose.position),
			input: InputDataType::Tip {
				data: Tip {
					pose: self.pose(tip.pose),
					chirality: tip.chirality,
					grip_pose: tip.grip_pose.map(|p| self.pose(p)),
					grip_surface_pose: tip.grip_surface_pose.map(|p| self.pose(p)),
					simulated_hand: tip.simulated_hand.as_ref().map(|h| self.localized_hand(h)),
				},
			},
		}
	}

	fn pointer(&self, pointer: &Pointer) -> Option<SpatialData> {
		let ray = self.handler_field.ray_march(Ray {
			origin: pointer.pose.position.into(),
			direction: Vec3::from(pointer.direction()),
			space: self.from.clone(),
		});
		Some(SpatialData {
			input: InputDataType::Pointer {
				data: Pointer {
					pose: self.pose(pointer.pose),
					deepest_point: ray.deepest_point_distance,
				},
			},
			distance: ray.min_distance,
		})
	}
}

/// closest any part of the hand gets to the field
///
/// the fingertip weighting is a heuristic for *ordering* handlers, which is
/// [`super::InputMethodHelper::order_handlers_and_captures`]'s job, not this one's
fn hand_distance(hand: &Hand) -> f32 {
	let finger = |f: &Finger| [f.tip, f.distal, f.intermediate, f.proximal, f.metacarpal];
	[
		hand.thumb.tip,
		hand.thumb.distal,
		hand.thumb.proximal,
		hand.thumb.metacarpal,
		hand.palm,
		hand.wrist,
	]
	.into_iter()
	.chain(hand.elbow)
	.chain(finger(&hand.index))
	.chain(finger(&hand.middle))
	.chain(finger(&hand.ring))
	.chain(finger(&hand.little))
	.map(|joint| joint.distance - joint.radius)
	.fold(f32::INFINITY, f32::min)
}

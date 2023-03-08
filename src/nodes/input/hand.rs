use crate::nodes::fields::Field;
use crate::nodes::spatial::Spatial;
use glam::{vec3a, Mat4};
use stardust_xr::schemas::flat::{Hand as FlatHand, InputDataType, Joint};
use std::sync::Arc;

use super::{DistanceLink, InputSpecialization};

#[derive(Debug, Default)]
pub struct Hand {
	pub base: FlatHand,
}
impl InputSpecialization for Hand {
	fn compare_distance(&self, space: &Arc<Spatial>, field: &Field) -> f32 {
		self.true_distance(space, field).abs()
	}
	fn true_distance(&self, space: &Arc<Spatial>, field: &Field) -> f32 {
		let mut min_distance = f32::MAX;

		for tip in [
			&self.base.thumb.tip.position,
			&self.base.index.tip.position,
			&self.base.middle.tip.position,
			&self.base.ring.tip.position,
			&self.base.little.tip.position,
		] {
			min_distance = min_distance.min(field.distance(space, vec3a(tip.x, tip.y, tip.z)));
		}

		min_distance
	}
	fn serialize(
		&self,
		_distance_link: &DistanceLink,
		local_to_handler_matrix: Mat4,
	) -> InputDataType {
		let mut hand = self.base;
		let mut joints: Vec<&mut Joint> = Vec::new();

		joints.extend([&mut hand.palm, &mut hand.wrist]);
		if let Some(elbow) = &mut hand.elbow {
			joints.push(elbow);
		}
		for finger in [
			&mut hand.index,
			&mut hand.middle,
			&mut hand.ring,
			&mut hand.little,
		] {
			joints.extend([
				&mut finger.tip,
				&mut finger.distal,
				&mut finger.intermediate,
				&mut finger.proximal,
				&mut finger.metacarpal,
			]);
		}
		joints.extend([
			&mut hand.thumb.tip,
			&mut hand.thumb.distal,
			&mut hand.thumb.proximal,
			&mut hand.thumb.metacarpal,
		]);

		for joint in joints {
			let joint_matrix = local_to_handler_matrix
				* Mat4::from_rotation_translation(joint.rotation.into(), joint.position.into());
			let (_, rotation, position) = joint_matrix.to_scale_rotation_translation();
			joint.position = position.into();
			joint.rotation = rotation.into();
		}

		InputDataType::Hand(Box::new(hand))
	}
}

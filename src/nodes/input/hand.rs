use super::{DistanceLink, Finger, Hand, InputDataTrait, Joint, Thumb};
use crate::nodes::fields::Field;
use crate::nodes::spatial::Spatial;
use glam::{vec3a, Mat4, Quat};
use std::sync::Arc;

impl Default for Joint {
	fn default() -> Self {
		Joint {
			position: [0.0; 3].into(),
			rotation: Quat::IDENTITY.into(),
			radius: 0.0,
			distance: 0.0,
		}
	}
}
impl Default for Finger {
	fn default() -> Self {
		Finger {
			tip: Default::default(),
			distal: Default::default(),
			intermediate: Default::default(),
			proximal: Default::default(),
			metacarpal: Default::default(),
		}
	}
}
impl Default for Thumb {
	fn default() -> Self {
		Thumb {
			tip: Default::default(),
			distal: Default::default(),
			proximal: Default::default(),
			metacarpal: Default::default(),
		}
	}
}
impl Default for Hand {
	fn default() -> Self {
		Hand {
			right: Default::default(),
			thumb: Default::default(),
			index: Default::default(),
			middle: Default::default(),
			ring: Default::default(),
			little: Default::default(),
			palm: Default::default(),
			wrist: Default::default(),
			elbow: Default::default(),
		}
	}
}

impl InputDataTrait for Hand {
	fn compare_distance(&self, space: &Arc<Spatial>, field: &Field) -> f32 {
		self.true_distance(space, field).abs()
	}
	fn true_distance(&self, space: &Arc<Spatial>, field: &Field) -> f32 {
		let mut min_distance = f32::MAX;

		for tip in [
			&self.thumb.tip.position,
			&self.index.tip.position,
			&self.middle.tip.position,
			&self.ring.tip.position,
			&self.little.tip.position,
		] {
			min_distance = min_distance.min(field.distance(space, vec3a(tip.x, tip.y, tip.z)));
		}

		min_distance
	}
	fn update_to(&mut self, distance_link: &DistanceLink, local_to_handler_matrix: Mat4) {
		let mut joints: Vec<&mut Joint> = Vec::new();

		joints.extend([&mut self.palm, &mut self.wrist]);
		if let Some(elbow) = &mut self.elbow {
			joints.push(elbow);
		}
		for finger in [
			&mut self.index,
			&mut self.middle,
			&mut self.ring,
			&mut self.little,
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
			&mut self.thumb.tip,
			&mut self.thumb.distal,
			&mut self.thumb.proximal,
			&mut self.thumb.metacarpal,
		]);

		for joint in joints {
			let joint_matrix = local_to_handler_matrix
				* Mat4::from_rotation_translation(joint.rotation.into(), joint.position.into());
			let (_, rotation, position) = joint_matrix.to_scale_rotation_translation();
			joint.position = position.into();
			joint.rotation = rotation.into();
			joint.distance = distance_link
				.handler
				.field
				.distance(&distance_link.handler.spatial, position.into());
		}
	}
}

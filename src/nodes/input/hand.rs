use crate::nodes::fields::Field;
use crate::nodes::spatial::Spatial;
use glam::{vec3a, Mat4};
use libstardustxr::schemas::input_hand::HandT;
use libstardustxr::schemas::{common::JointT, input::InputDataRaw};
use std::sync::Arc;

use super::{DistanceLink, InputSpecialization};

impl InputSpecialization for HandT {
	fn distance(&self, space: &Arc<Spatial>, field: &Field) -> f32 {
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
	fn serialize(
		&self,
		fbb: &mut flatbuffers::FlatBufferBuilder,
		_distance_link: &DistanceLink,
		local_to_handler_matrix: Mat4,
	) -> (
		InputDataRaw,
		flatbuffers::WIPOffset<flatbuffers::UnionWIPOffset>,
	) {
		let mut hand = self.clone();
		let mut joints: Vec<&mut JointT> = Vec::new();

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
			let rotation: mint::Quaternion<f32> = joint.rotation.clone().into();
			let position: mint::Vector3<f32> = joint.position.clone().into();
			let joint_matrix = Mat4::from_rotation_translation(rotation.into(), position.into())
				* local_to_handler_matrix;
			let (_, rotation, position) = joint_matrix.to_scale_rotation_translation();
			let rotation: mint::Quaternion<f32> = rotation.into();
			let position: mint::Vector3<f32> = position.into();
			joint.position = position.into();
			joint.rotation = rotation.into();
		}

		(InputDataRaw::Hand, hand.pack(fbb).as_union_value())
	}
	fn serialize_datamap(&self) -> Vec<u8> {
		let mut fbb = flexbuffers::Builder::default();
		let mut map = fbb.start_map();
		map.push("right", self.right);
		map.end_map();
		fbb.view().to_vec()
	}
}

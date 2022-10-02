use super::{DistanceLink, InputSpecialization};
use crate::nodes::fields::Field;
use crate::nodes::spatial::Spatial;
use glam::{vec3a, Mat4};
use portable_atomic::AtomicF32;
use stardust_xr_schemas::input::InputDataRaw;
use stardust_xr_schemas::input_tip;
use std::sync::atomic::Ordering;
use std::sync::Arc;

#[derive(Default)]
pub struct Tip {
	pub radius: AtomicF32,
	pub grab: AtomicF32,
	pub select: AtomicF32,
}

impl InputSpecialization for Tip {
	fn distance(&self, space: &Arc<Spatial>, field: &Field) -> f32 {
		field.distance(space, vec3a(0.0, 0.0, 0.0))
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
		let (_, orientation, origin) = local_to_handler_matrix.to_scale_rotation_translation();

		let origin: mint::Vector3<f32> = origin.into();
		let orientation: mint::Quaternion<f32> = orientation.into();

		let tip = input_tip::Tip::create(
			fbb,
			&input_tip::TipArgs {
				origin: Some(&origin.into()),
				orientation: Some(&orientation.into()),
				radius: self.radius.load(Ordering::Relaxed),
			},
		);
		(InputDataRaw::Tip, tip.as_union_value())
	}
	fn serialize_datamap(&self) -> Vec<u8> {
		let mut fbb = flexbuffers::Builder::default();
		let mut map = fbb.start_map();
		map.push("grab", self.grab.load(Ordering::Relaxed));
		map.push("select", self.select.load(Ordering::Relaxed));
		map.end_map();
		fbb.view().to_vec()
	}
}

use super::{DistanceLink, InputSpecialization};
use crate::nodes::fields::{ray_march, Field, Ray, RayMarchResult};
use crate::nodes::spatial::Spatial;
use glam::{vec3, Mat4};
use stardust_xr_schemas::input::InputDataRaw;
use stardust_xr_schemas::input_pointer;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Default)]
pub struct Pointer {
	grab: AtomicBool,
	select: AtomicBool,
}
// impl Default for Pointer {
// 	fn default() -> Self {
// 		Pointer {
// 			grab: Default::default(),
// 			select: Default::default(),
// 		}
// 	}
// }
impl Pointer {
	fn ray_march(&self, space: &Arc<Spatial>, field: &Field) -> RayMarchResult {
		ray_march(
			Ray {
				origin: vec3(0_f32, 0_f32, 0_f32),
				direction: vec3(0_f32, 0_f32, 1_f32),
				space: space.clone(),
			},
			field,
		)
	}
}

impl InputSpecialization for Pointer {
	fn distance(&self, space: &Arc<Spatial>, field: &Field) -> f32 {
		self.ray_march(space, field).distance
	}
	fn serialize(
		&self,
		fbb: &mut flatbuffers::FlatBufferBuilder,
		distance_link: &DistanceLink,
		local_to_handler_matrix: Mat4,
	) -> (
		InputDataRaw,
		flatbuffers::WIPOffset<flatbuffers::UnionWIPOffset>,
	) {
		let (_, orientation, origin) = local_to_handler_matrix.to_scale_rotation_translation();
		let direction = local_to_handler_matrix.transform_vector3(vec3(0_f32, 0_f32, 1_f32));
		let ray_march = self.ray_march(
			&distance_link.method.spatial,
			&distance_link.handler.field.upgrade().unwrap(),
		);
		let deepest_point = (direction * ray_march.deepest_point_distance) + origin;

		let origin: mint::Vector3<f32> = origin.into();
		let orientation: mint::Quaternion<f32> = orientation.into();
		let deepest_point: mint::Vector3<f32> = deepest_point.into();

		let pointer = input_pointer::Pointer::create(
			fbb,
			&input_pointer::PointerArgs {
				origin: Some(&origin.into()),
				orientation: Some(&orientation.into()),
				deepest_point: Some(&deepest_point.into()),
			},
		);
		(InputDataRaw::Pointer, pointer.as_union_value())
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
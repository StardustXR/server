use super::field::{ray_march, Field, Ray, RayMarchResult};
use super::input::{DistanceLink, InputSpecializationTrait};
use super::spatial::Spatial;
use glam::{vec3, vec3a, Mat4};
use libstardustxr::schemas::common;
use libstardustxr::schemas::input::InputDataRaw;
use libstardustxr::schemas::input_pointer;
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

impl InputSpecializationTrait for Pointer {
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
		let origin = local_to_handler_matrix.transform_point3a(vec3a(0_f32, 0_f32, 0_f32));
		let direction = local_to_handler_matrix.transform_vector3a(vec3a(0_f32, 0_f32, 1_f32));
		let ray_march = self.ray_march(
			&distance_link.method.upgrade().unwrap().spatial,
			&distance_link
				.handler
				.upgrade()
				.unwrap()
				.field
				.upgrade()
				.unwrap(),
		);
		let deepest_point = (direction * ray_march.deepest_point_distance) + origin;

		let pointer = input_pointer::Pointer::create(
			fbb,
			&input_pointer::PointerArgs {
				origin: Some(&common::Vec3::new(origin.x, origin.y, origin.z)),
				direction: Some(&common::Vec3::new(direction.x, direction.y, direction.z)),
				tilt: 0_f32,
				deepest_point: Some(&common::Vec3::new(
					deepest_point.x,
					deepest_point.y,
					deepest_point.z,
				)),
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

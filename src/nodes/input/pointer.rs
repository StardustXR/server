use super::{DistanceLink, InputSpecialization};
use crate::nodes::fields::{ray_march, Field, Ray, RayMarchResult};
use crate::nodes::spatial::Spatial;
use glam::{vec3, Mat4};
use stardust_xr::schemas::flat::{InputDataType, Pointer as FlatPointer};
use std::sync::Arc;

#[derive(Default)]
pub struct Pointer {}
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
		self.ray_march(space, field).min_distance
	}
	fn serialize(
		&self,
		distance_link: &DistanceLink,
		local_to_handler_matrix: Mat4,
	) -> InputDataType {
		let (_, orientation, origin) = local_to_handler_matrix.to_scale_rotation_translation();
		let direction = local_to_handler_matrix.transform_vector3(vec3(0_f32, 0_f32, 1_f32));
		let ray_march = self.ray_march(
			&distance_link.method.spatial,
			&distance_link.handler.field.upgrade().unwrap(),
		);
		let deepest_point = (direction * ray_march.deepest_point_distance) + origin;

		InputDataType::Pointer(FlatPointer {
			origin: origin.into(),
			orientation: orientation.into(),
			deepest_point: deepest_point.into(),
		})
	}
}

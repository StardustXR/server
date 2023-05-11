use super::{DistanceLink, InputSpecialization};
use crate::nodes::fields::{Field, Ray, RayMarchResult};
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
		field.ray_march(Ray {
			origin: vec3(0_f32, 0_f32, 0_f32),
			direction: vec3(0_f32, 0_f32, 1_f32),
			space: space.clone(),
		})
	}
}

impl InputSpecialization for Pointer {
	fn compare_distance(&self, space: &Arc<Spatial>, field: &Field) -> f32 {
		let ray_info = self.ray_march(space, field);
		ray_info
			.deepest_point_distance
			.hypot(ray_info.min_distance.recip())
	}
	fn true_distance(&self, space: &Arc<Spatial>, field: &Field) -> f32 {
		let ray_info = self.ray_march(space, field);
		ray_info.min_distance
	}
	fn serialize(
		&self,
		distance_link: &DistanceLink,
		local_to_handler_matrix: Mat4,
	) -> InputDataType {
		let (_, orientation, origin) = local_to_handler_matrix.to_scale_rotation_translation();
		let direction = local_to_handler_matrix.transform_vector3(vec3(0_f32, 0_f32, 1_f32));
		let ray_march = self.ray_march(&distance_link.method.spatial, &distance_link.handler.field);
		let deepest_point = (direction * ray_march.deepest_point_distance) + origin;

		InputDataType::Pointer(FlatPointer {
			origin: origin.into(),
			orientation: orientation.into(),
			deepest_point: deepest_point.into(),
		})
	}
}

use super::{DistanceLink, InputDataTrait, Pointer};
use crate::nodes::{
	fields::{Field, Ray, RayMarchResult},
	spatial::Spatial,
};
use glam::{vec3, Mat4, Quat};
use std::sync::{Arc, Weak};

impl Default for Pointer {
	fn default() -> Self {
		Pointer {
			origin: [0.0; 3].into(),
			orientation: Quat::IDENTITY.into(),
			deepest_point: [0.0; 3].into(),
		}
	}
}
impl Pointer {
	fn ray_march(&self, method_space: &Arc<Spatial>, field: &Field) -> RayMarchResult {
		field.ray_march(Ray {
			origin: vec3(0.0, 0.0, 0.0),
			direction: vec3(0.0, 0.0, -1.0),
			space: Spatial::new(
				Weak::new(),
				Some(method_space.clone()),
				Mat4::from_rotation_translation(self.orientation.into(), self.origin.into()),
			),
		})
	}
}
impl InputDataTrait for Pointer {
	fn compare_distance(&self, space: &Arc<Spatial>, field: &Field) -> f32 {
		let ray_info = self.ray_march(space, field);
		if ray_info.min_distance > 0.0 {
			ray_info.deepest_point_distance + 1000.0
		} else {
			ray_info
				.deepest_point_distance
				.hypot(0.001 / ray_info.min_distance)
		}
	}
	fn true_distance(&self, space: &Arc<Spatial>, field: &Field) -> f32 {
		let ray_info = self.ray_march(space, field);
		ray_info.min_distance
	}
	fn update_to(&mut self, distance_link: &DistanceLink, mut local_to_handler_matrix: Mat4) {
		local_to_handler_matrix =
			Mat4::from_rotation_translation(self.orientation.into(), self.origin.into())
				* local_to_handler_matrix;
		let (_, orientation, origin) = local_to_handler_matrix.to_scale_rotation_translation();

		let ray_march = self.ray_march(&distance_link.method.spatial, &distance_link.handler.field);
		let direction = local_to_handler_matrix
			.transform_vector3(vec3(0.0, 0.0, -1.0))
			.normalize();
		let deepest_point = (direction * ray_march.deepest_point_distance) + origin;

		self.origin = origin.into();
		self.orientation = orientation.into();
		self.deepest_point = deepest_point.into();
	}
}

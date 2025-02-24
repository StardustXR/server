use super::{InputDataTrait, InputHandler, InputMethod, Pointer};
use crate::nodes::{
	fields::{Field, FieldTrait, Ray, RayMarchResult},
	spatial::Spatial,
};
use glam::{Mat4, Quat, vec3};
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
	fn distance(&self, space: &Arc<Spatial>, field: &Field) -> f32 {
		let ray_info = self.ray_march(space, field);
		ray_info.min_distance
	}
	fn transform(&mut self, method: &InputMethod, handler: &InputHandler) {
		let local_to_handler_matrix =
			Mat4::from_rotation_translation(self.orientation.into(), self.origin.into())
				* Spatial::space_to_space_matrix(Some(&method.spatial), Some(&handler.spatial));
		let (_, orientation, origin) = local_to_handler_matrix.to_scale_rotation_translation();

		let ray_march = self.ray_march(&method.spatial, &handler.field);
		let direction = local_to_handler_matrix
			.transform_vector3(vec3(0.0, 0.0, -1.0))
			.normalize();
		let deepest_point = (direction * ray_march.deepest_point_distance) + origin;

		self.origin = origin.into();
		self.orientation = orientation.into();
		self.deepest_point = deepest_point.into();
	}
}

use super::{DistanceLink, InputDataTrait, Tip};
use crate::nodes::{fields::Field, spatial::Spatial};
use glam::{Mat4, Quat};
use std::sync::Arc;

impl Default for Tip {
	fn default() -> Self {
		Tip {
			origin: [0.0; 3].into(),
			orientation: Quat::IDENTITY.into(),
		}
	}
}
impl InputDataTrait for Tip {
	fn compare_distance(&self, space: &Arc<Spatial>, field: &Field) -> f32 {
		field.distance(space, self.origin.into()).abs()
	}
	fn true_distance(&self, space: &Arc<Spatial>, field: &Field) -> f32 {
		field.distance(space, self.origin.into())
	}
	fn update_to(&mut self, _distance_link: &DistanceLink, mut local_to_handler_matrix: Mat4) {
		local_to_handler_matrix *=
			Mat4::from_rotation_translation(self.orientation.into(), self.origin.into());
		let (_, orientation, origin) = local_to_handler_matrix.to_scale_rotation_translation();
		self.origin = origin.into();
		self.orientation = orientation.into();
	}
}

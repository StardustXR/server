use super::{InputDataTrait, InputHandler, InputMethod, Tip};
use crate::nodes::{
	fields::{Field, FieldTrait},
	spatial::Spatial,
};
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
	fn distance(&self, space: &Arc<Spatial>, field: &Field) -> f32 {
		field.distance(space, self.origin.into())
	}
	fn transform(&mut self, method: &InputMethod, handler: &InputHandler) {
		let local_to_handler_matrix =
			Spatial::space_to_space_matrix(Some(&method.spatial), Some(&handler.spatial))
				* Mat4::from_rotation_translation(self.orientation.into(), self.origin.into());
		let (_, orientation, origin) = local_to_handler_matrix.to_scale_rotation_translation();
		self.origin = origin.into();
		self.orientation = orientation.into();
	}
}

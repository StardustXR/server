use crate::nodes::{
	input::{pointer::Pointer, InputMethod, InputType},
	spatial::Spatial,
};
use glam::{vec3, Mat4};
use std::sync::{Arc, Weak};
use stereokit::{input::Ray, StereoKit};

pub struct MousePointer {
	pointer: Arc<InputMethod>,
}
impl MousePointer {
	pub fn new() -> Self {
		MousePointer {
			pointer: InputMethod::new(
				Spatial::new(Weak::new(), None, Mat4::IDENTITY),
				InputType::Pointer(Pointer::default()),
			),
		}
	}
	pub fn update(&self, sk: &StereoKit) {
		if let Some(ray) = Ray::from_mouse(sk.input_mouse()) {
			self.pointer.spatial.set_local_transform_components(
				None,
				Some(ray.pos.into()),
				Some(glam::Quat::from_rotation_arc(
					vec3(0.0, 0.0, 1.0),
					ray.dir.into(),
				)),
				None,
			);
		}
	}
}

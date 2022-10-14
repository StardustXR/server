use crate::nodes::{
	input::{pointer::Pointer, InputMethod, InputType},
	spatial::Spatial,
};
use glam::{vec3, Mat4};
use stardust_xr::{schemas::flat::Datamap, values::Transform};
use std::sync::{Arc, Weak};
use stereokit::{
	input::{ButtonState, Key, Ray},
	StereoKit,
};

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
				Transform {
					position: Some(ray.pos),
					rotation: Some(
						glam::Quat::from_rotation_arc(vec3(0.0, 0.0, 1.0), ray.dir.into()).into(),
					),
					scale: None,
				},
			);
		}
		let mut fbb = flexbuffers::Builder::default();
		let mut map = fbb.start_map();
		map.push(
			"select",
			if sk.input_key(Key::MouseLeft).contains(ButtonState::Active) {
				1.0f32
			} else {
				0.0f32
			},
		);
		map.push(
			"grab",
			if sk.input_key(Key::MouseRight).contains(ButtonState::Active) {
				1.0f32
			} else {
				0.0f32
			},
		);
		map.end_map();
		*self.pointer.datamap.lock() = Datamap::new(fbb.take_buffer()).ok();
	}
}

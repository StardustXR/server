use crate::{
	core::client::INTERNAL_CLIENT,
	nodes::{
		fields::Ray,
		input::{InputDataType, InputMethod, Pointer, INPUT_HANDLER_REGISTRY},
		spatial::Spatial,
		Node,
	},
};
use color_eyre::eyre::Result;
use glam::{vec3, Mat4};
use serde::{Deserialize, Serialize};
use stardust_xr::values::Datamap;
use std::sync::Arc;
use stereokit_rust::system::Input;

#[derive(Default, Deserialize, Serialize)]
pub struct EyeDatamap {
	eye: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct KeyboardEvent {
	pub keyboard: String,
	pub keymap: Option<String>,
	pub keys_up: Option<Vec<u32>>,
	pub keys_down: Option<Vec<u32>>,
}

pub struct EyePointer {
	spatial: Arc<Spatial>,
	pointer: Arc<InputMethod>,
}
impl EyePointer {
	pub fn new() -> Result<Self> {
		let node = Node::generate(&INTERNAL_CLIENT, false).add_to_scenegraph()?;
		let spatial = Spatial::add_to(&node, None, Mat4::IDENTITY, false);
		let pointer = InputMethod::add_to(
			&node,
			InputDataType::Pointer(Pointer::default()),
			Datamap::from_typed(EyeDatamap::default())?,
		)
		.unwrap();

		Ok(EyePointer { spatial, pointer })
	}
	pub fn update(&self) {
		let ray = Input::get_eyes();
		self.spatial.set_local_transform(
			Mat4::from_rotation_translation(ray.orientation.into(), ray.position.into()),
		);
		{
			// Set pointer input datamap
			*self.pointer.datamap.lock() = Datamap::from_typed(EyeDatamap { eye: 2 }).unwrap();
		}

		// send input to all the input handlers that are the closest to the ray as possible
		let rx = INPUT_HANDLER_REGISTRY
			.get_valid_contents()
			.into_iter()
			// filter out all the disabled handlers
			.filter(|handler| {
				let Some(node) = handler.spatial.node() else {
					return false;
				};
				node.enabled()
			})
			// ray march to all the enabled handlers' fields
			.map(|handler| {
				let result = handler.field.ray_march(Ray {
					origin: vec3(0.0, 0.0, 0.0),
					direction: vec3(0.0, 0.0, -1.0),
					space: self.spatial.clone(),
				});
				(vec![handler], result)
			})
			// make sure the field isn't at the pointer origin and that it's being hit
			.filter(|(_, result)| result.deepest_point_distance > 0.01 && result.min_distance < 0.0)
			// .inspect(|(_, result)| {
			// 	dbg!(result);
			// })
			// now collect all handlers that are same distance if they're the closest
			.reduce(|(mut handlers_a, result_a), (handlers_b, result_b)| {
				if (result_a.deepest_point_distance - result_b.deepest_point_distance).abs() < 0.001
				{
					// distance is basically the same
					handlers_a.extend(handlers_b);
					(handlers_a, result_a)
				} else if result_a.deepest_point_distance < result_b.deepest_point_distance {
					(handlers_a, result_a)
				} else {
					(handlers_b, result_b)
				}
			})
			.map(|(rx, _)| rx)
			.unwrap_or_default();
		self.pointer.set_handler_order(rx.iter());
	}
}

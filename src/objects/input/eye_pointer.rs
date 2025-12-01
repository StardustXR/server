use crate::{
	core::client::INTERNAL_CLIENT,
	nodes::{
		Node, OwnedNode,
		fields::{FieldTrait, Ray},
		input::{INPUT_HANDLER_REGISTRY, InputDataType, InputMethod, Pointer},
		spatial::Spatial,
	},
};
use color_eyre::eyre::Result;
use glam::{Mat4, vec3};
use serde::{Deserialize, Serialize};
use stardust_xr::values::Datamap;
use std::sync::Arc;

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
	node: OwnedNode,
	spatial: Arc<Spatial>,
	pointer: Arc<InputMethod>,
}
impl EyePointer {
	pub fn new() -> Result<Self> {
		let node = Node::generate(&INTERNAL_CLIENT, false).add_to_scenegraph_owned()?;
		let spatial = Spatial::add_to(&node.0, None, Mat4::IDENTITY);
		let pointer = InputMethod::add_to(
			&node.0,
			InputDataType::Pointer(Pointer::default()),
			Datamap::from_typed(EyeDatamap::default())?,
		)
		.unwrap();

		Ok(EyePointer {
			node,
			spatial,
			pointer,
		})
	}
}

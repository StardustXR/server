#![allow(clippy::needless_question_mark)]

mod hand;
mod handler;
mod link;
mod method;
mod pointer;
mod tip;

pub use handler::*;
pub use link::*;
pub use method::*;

use super::Aspect;
use super::AspectIdentifier;
use super::fields::Field;
use super::spatial::Spatial;
use crate::core::Id;
use crate::core::error::Result;
use crate::nodes::spatial::SPATIAL_ASPECT_ALIAS_INFO;
use crate::nodes::spatial::SPATIAL_REF_ASPECT_ALIAS_INFO;
use crate::{core::client::Client, nodes::Node};
use crate::{core::registry::Registry, nodes::spatial::Transform};
use stardust_xr_wire::values::Datamap;
use std::sync::Arc;

static INPUT_METHOD_REGISTRY: Registry<InputMethod> = Registry::new();
pub static INPUT_HANDLER_REGISTRY: Registry<InputHandler> = Registry::new();

stardust_xr_server_codegen::codegen_input_protocol!();

impl AspectIdentifier for InputHandler {
	impl_aspect_for_input_handler_aspect_id! {}
}
impl Aspect for InputHandler {
	impl_aspect_for_input_handler_aspect! {}
}
impl AspectIdentifier for InputMethod {
	impl_aspect_for_input_method_aspect_id! {}
}
impl Aspect for InputMethod {
	impl_aspect_for_input_method_aspect! {}
}
impl AspectIdentifier for InputMethodRef {
	impl_aspect_for_input_method_ref_aspect_id! {}
}
impl Aspect for InputMethodRef {
	impl_aspect_for_input_method_ref_aspect! {}
}

pub trait InputDataTrait {
	fn transform(&mut self, method: &InputMethod, handler: &InputHandler);
	fn distance(&self, space: &Arc<Spatial>, field: &Field) -> f32;
}
impl InputDataTrait for InputDataType {
	fn transform(&mut self, method: &InputMethod, handler: &InputHandler) {
		match self {
			InputDataType::Pointer(i) => i.transform(method, handler),
			InputDataType::Hand(i) => i.transform(method, handler),
			InputDataType::Tip(i) => i.transform(method, handler),
		}
	}

	fn distance(&self, space: &Arc<Spatial>, field: &Field) -> f32 {
		match self {
			InputDataType::Pointer(i) => i.distance(space, field),
			InputDataType::Hand(i) => i.distance(space, field),
			InputDataType::Tip(i) => i.distance(space, field),
		}
	}
}

impl InterfaceAspect for Interface {
	#[doc = "Create an input method node"]
	fn create_input_method(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		id: Id,
		parent: Arc<Node>,
		transform: Transform,
		initial_data: InputDataType,
		datamap: Datamap,
	) -> Result<()> {
		let parent = parent.get_aspect::<Spatial>()?;
		let transform = transform.to_mat4(true, true, true);

		let node = Node::from_id(&calling_client, id, true).add_to_scenegraph()?;
		Spatial::add_to(&node, Some(parent.clone()), transform);
		InputMethod::add_to(&node, initial_data, datamap)?;
		Ok(())
	}

	#[doc = "Create an input handler node"]
	fn create_input_handler(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		id: Id,
		parent: Arc<Node>,
		transform: Transform,
		field: Arc<Node>,
	) -> Result<()> {
		let parent = parent.get_aspect::<Spatial>()?;
		let transform = transform.to_mat4(true, true, true);
		let field = field.get_aspect::<Field>()?;

		let node = Node::from_id(&calling_client, id, true).add_to_scenegraph()?;
		Spatial::add_to(&node, Some(parent.clone()), transform);
		InputHandler::add_to(&node, &field)?;
		Ok(())
	}
}

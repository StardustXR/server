#![allow(clippy::needless_question_mark)]

mod hand;
mod handler;
mod method;
mod pointer;
mod tip;

use bevy::tasks::ComputeTaskPool;
use bevy::tasks::ParallelSlice;
pub use handler::*;
pub use method::*;
use tracing::debug_span;

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
use stardust_xr::values::Datamap;
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

#[tracing::instrument(level = "debug")]
pub fn process_input() {
	// Iterate over all valid input methods
	let methods = INPUT_METHOD_REGISTRY
		.get_valid_contents()
		.into_iter()
		.filter(|method| {
			let Some(node) = method.spatial.node() else {
				return false;
			};
			node.enabled()
		});
	INPUT_HANDLER_REGISTRY
		.get_valid_contents()
		.into_iter()
		.par_splat_map(ComputeTaskPool::get(), None, |_, handlers| {
			for handler in handlers {
				let _span = debug_span!("handle input handler").entered();
				for method_alias in handler.method_aliases.get_aliases() {
					method_alias.set_enabled(false);
				}

				let Some(handler_node) = handler.spatial.node() else {
					continue;
				};
				if !handler_node.enabled() {
					continue;
				}
				if let Some(handler_field_node) = handler.field.spatial.node()
					&& !handler_field_node.enabled()
				{
					continue;
				};

				let ser_span = debug_span!("serializing input").entered();
				let (methods, datas) = methods
					.clone()
					// filter out methods without the handler in their handler order
					.filter(|a| {
						a.handler_order
							.lock()
							.iter()
							.any(|h| h.ptr_eq(&Arc::downgrade(handler)))
					})
					// filter out methods without the proper alias
					.filter_map(|m| {
						Some((
							handler
								.method_aliases
								.get_from_original_node(m.spatial.node.clone())?,
							m,
						))
					})
					// make sure the input method alias is enabled
					.inspect(|(a, _)| {
						a.set_enabled(true);
					})
					// serialize the data
					.map(|(a, m)| (a.clone(), m.serialize(a.get_id(), handler)))
					.unzip::<_, _, Vec<_>, Vec<_>>();
				drop(ser_span);

				let _span = debug_span!("client input").entered();
				let _ = input_handler_client::input(&handler_node, &methods, &datas);
			}
		});
	for method in methods {
		method.cull_capture_attempts();
	}
}

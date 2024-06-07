mod hand;
mod handler;
mod method;
mod pointer;
mod tip;

pub use handler::*;
pub use method::*;
use rustc_hash::FxHashMap;

use super::fields::Field;
use super::spatial::Spatial;
use crate::create_interface;
use crate::nodes::spatial::SPATIAL_ASPECT_ALIAS_INFO;
use crate::nodes::spatial::SPATIAL_REF_ASPECT_ALIAS_INFO;
use crate::{core::client::Client, nodes::Node};
use crate::{core::registry::Registry, nodes::spatial::Transform};
use color_eyre::eyre::Result;
use glam::Mat4;
use stardust_xr::values::Datamap;
use std::sync::{Arc, Weak};
use tracing::{debug_span, instrument};

static INPUT_METHOD_REGISTRY: Registry<InputMethod> = Registry::new();
pub static INPUT_HANDLER_REGISTRY: Registry<InputHandler> = Registry::new();

stardust_xr_server_codegen::codegen_input_protocol!();

pub struct InputLink {
	method: Arc<InputMethod>,
	handler: Arc<InputHandler>,
}
impl InputLink {
	fn from(method: Arc<InputMethod>, handler: Arc<InputHandler>) -> Self {
		InputLink { method, handler }
	}

	#[instrument(level = "debug", skip(self))]
	fn serialize(&self, id: u64, order: u32, captured: bool, datamap: Datamap) -> InputData {
		let mut input = self.method.data.lock().clone();
		input.update_to(
			self,
			Spatial::space_to_space_matrix(Some(&self.method.spatial), Some(&self.handler.spatial)),
		);

		InputData {
			id,
			input,
			distance: self.method.distance(&self.handler.field),
			datamap,
			order,
			captured,
		}
	}
}
pub trait InputDataTrait {
	fn distance(&self, space: &Arc<Spatial>, field: &Field) -> f32;
	fn update_to(&mut self, input_link: &InputLink, local_to_handler_matrix: Mat4);
}
impl InputDataTrait for InputDataType {
	fn distance(&self, space: &Arc<Spatial>, field: &Field) -> f32 {
		match self {
			InputDataType::Pointer(i) => i.distance(space, field),
			InputDataType::Hand(i) => i.distance(space, field),
			InputDataType::Tip(i) => i.distance(space, field),
		}
	}

	fn update_to(&mut self, input_link: &InputLink, local_to_handler_matrix: Mat4) {
		match self {
			InputDataType::Pointer(i) => i.update_to(input_link, local_to_handler_matrix),
			InputDataType::Hand(i) => i.update_to(input_link, local_to_handler_matrix),
			InputDataType::Tip(i) => i.update_to(input_link, local_to_handler_matrix),
		}
	}
}

create_interface!(InputInterface);
pub struct InputInterface;
impl InterfaceAspect for InputInterface {
	#[doc = "Create an input method node"]
	fn create_input_method(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		id: u64,
		parent: Arc<Node>,
		transform: Transform,
		initial_data: InputDataType,
		datamap: Datamap,
	) -> Result<()> {
		let parent = parent.get_aspect::<Spatial>()?;
		let transform = transform.to_mat4(true, true, true);

		let node = Node::from_id(&calling_client, id, true).add_to_scenegraph()?;
		Spatial::add_to(&node, Some(parent.clone()), transform, false);
		InputMethod::add_to(&node, initial_data, datamap)?;
		Ok(())
	}

	#[doc = "Create an input handler node"]
	fn create_input_handler(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		id: u64,
		parent: Arc<Node>,
		transform: Transform,
		field: Arc<Node>,
	) -> Result<()> {
		let parent = parent.get_aspect::<Spatial>()?;
		let transform = transform.to_mat4(true, true, true);
		let field = field.get_aspect::<Field>()?;

		let node = Node::from_id(&calling_client, id, true).add_to_scenegraph()?;
		Spatial::add_to(&node, Some(parent.clone()), transform, false);
		InputHandler::add_to(&node, &field)?;
		Ok(())
	}
}

#[tracing::instrument(level = "debug")]
pub fn process_input() {
	// Iterate over all valid input methods
	let methods = debug_span!("Get valid methods").in_scope(|| {
		INPUT_METHOD_REGISTRY
			.get_valid_contents()
			.into_iter()
			.filter(|method| {
				let Some(node) = method.spatial.node() else {
					return false;
				};
				node.enabled()
			})
	});
	for handler in INPUT_HANDLER_REGISTRY.get_valid_contents() {
		for method_alias in handler.method_aliases.get_aliases() {
			method_alias.set_enabled(false);
		}
	}
	let mut handler_input: FxHashMap<u64, (Arc<Node>, Vec<Arc<Node>>, Vec<InputData>)> =
		Default::default();
	// const LIMIT: usize = 50;
	for method in methods {
		debug_span!("Process input method").in_scope(|| {
			// Get all valid input handlers and convert them to InputLink objects
			let input_links: Vec<InputLink> = debug_span!("Generate input links").in_scope(|| {
				method
					.handler_order
					.lock()
					.iter()
					.filter_map(Weak::upgrade)
					.filter(|handler| {
						let Some(node) = handler.spatial.node() else {
							return false;
						};
						node.enabled()
					})
					.map(|handler| InputLink::from(method.clone(), handler))
					.collect()
			});

			// Iterate over the distance links and send input to them
			for (i, input_link) in input_links.into_iter().enumerate() {
				let handler = input_link.handler.spatial.node().unwrap();
				if !handler_input.contains_key(&handler.id) {
					handler_input.insert(handler.id, (handler.clone(), Vec::new(), Vec::new()));
				}
				let (_, methods, datas) = handler_input.get_mut(&handler.id).unwrap();

				let method_alias = input_link
					.handler
					.method_aliases
					.get(input_link.method.as_ref())
					.unwrap();
				method_alias.set_enabled(true);
				datas.push(input_link.serialize(
					method_alias.id,
					i as u32,
					method.captures.contains(&input_link.handler),
					method.datamap.lock().clone(),
				));
				methods.push(method_alias);
			}
			method.capture_requests.clear();
		});
	}

	for (_, (handler, methods, data)) in handler_input {
		let _ = input_handler_client::input(&handler, &methods, &data);
	}
}

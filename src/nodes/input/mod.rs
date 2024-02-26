mod hand;
mod handler;
mod method;
mod pointer;
mod tip;

pub use handler::*;
pub use method::*;

use super::fields::Field;
use super::spatial::Spatial;
use crate::create_interface;
use crate::nodes::alias::Alias;
use crate::{core::client::Client, nodes::Node};
use crate::{core::registry::Registry, nodes::spatial::Transform};
use color_eyre::eyre::Result;
use glam::Mat4;
use portable_atomic::Ordering;
use stardust_xr::values::Datamap;
use std::sync::{Arc, Weak};
use tracing::{debug_span, instrument};

static INPUT_METHOD_REGISTRY: Registry<InputMethod> = Registry::new();
static INPUT_HANDLER_REGISTRY: Registry<InputHandler> = Registry::new();

stardust_xr_server_codegen::codegen_input_protocol!();

pub struct DistanceLink {
	distance: f32,
	method: Arc<InputMethod>,
	handler: Arc<InputHandler>,
}
impl DistanceLink {
	fn from(method: Arc<InputMethod>, handler: Arc<InputHandler>) -> Self {
		DistanceLink {
			distance: method.compare_distance(&handler),
			method,
			handler,
		}
	}

	fn send_input(&self, order: u32, captured: bool, datamap: Datamap) {
		self.handler.send_input(order, captured, self, datamap);
	}
	#[instrument(level = "debug", skip(self))]
	fn serialize(&self, order: u32, captured: bool, datamap: Datamap) -> InputData {
		let mut input = self.method.data.lock().clone();
		input.update_to(
			self,
			Spatial::space_to_space_matrix(Some(&self.method.spatial), Some(&self.handler.spatial)),
		);

		InputData {
			uid: self.method.uid.clone(),
			input,
			distance: self.method.true_distance(&self.handler.field),
			datamap,
			order,
			captured,
		}
	}
}
pub trait InputDataTrait {
	fn compare_distance(&self, space: &Arc<Spatial>, field: &Field) -> f32;
	fn true_distance(&self, space: &Arc<Spatial>, field: &Field) -> f32;
	fn update_to(&mut self, distance_link: &DistanceLink, local_to_handler_matrix: Mat4);
}
impl InputDataTrait for InputDataType {
	fn compare_distance(&self, space: &Arc<Spatial>, field: &Field) -> f32 {
		match self {
			InputDataType::Pointer(i) => i.compare_distance(space, field),
			InputDataType::Hand(i) => i.compare_distance(space, field),
			InputDataType::Tip(i) => i.compare_distance(space, field),
		}
	}

	fn true_distance(&self, space: &Arc<Spatial>, field: &Field) -> f32 {
		match self {
			InputDataType::Pointer(i) => i.true_distance(space, field),
			InputDataType::Hand(i) => i.true_distance(space, field),
			InputDataType::Tip(i) => i.true_distance(space, field),
		}
	}

	fn update_to(&mut self, distance_link: &DistanceLink, local_to_handler_matrix: Mat4) {
		match self {
			InputDataType::Pointer(i) => i.update_to(distance_link, local_to_handler_matrix),
			InputDataType::Hand(i) => i.update_to(distance_link, local_to_handler_matrix),
			InputDataType::Tip(i) => i.update_to(distance_link, local_to_handler_matrix),
		}
	}
}

create_interface!(InputInterface, InputInterfaceAspect, "/input");
pub struct InputInterface;
impl InputInterfaceAspect for InputInterface {
	#[doc = "Create an input method node"]
	fn create_input_method(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		name: String,
		parent: Arc<Node>,
		transform: Transform,
		initial_data: InputDataType,
		datamap: Datamap,
	) -> Result<()> {
		let parent = parent.get_aspect::<Spatial>()?;
		let transform = transform.to_mat4(true, true, true);

		let node = Node::create_parent_name(&calling_client, "/input/method", &name, true)
			.add_to_scenegraph()?;
		Spatial::add_to(&node, Some(parent.clone()), transform, false);
		InputMethod::add_to(&node, initial_data, datamap)?;
		Ok(())
	}

	#[doc = "Create an input handler node"]
	fn create_input_handler(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		name: String,
		parent: Arc<Node>,
		transform: Transform,
		field: Arc<Node>,
	) -> Result<()> {
		let parent = parent.get_aspect::<Spatial>()?;
		let transform = transform.to_mat4(true, true, true);
		let field = field.get_aspect::<Field>()?;

		let node = Node::create_parent_name(&calling_client, "/input/handler", &name, true)
			.add_to_scenegraph()?;
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
			.filter(|method| *method.enabled.lock())
	});
	let handlers = INPUT_HANDLER_REGISTRY.get_valid_contents();
	const LIMIT: usize = 50;
	for method in methods {
		for alias in method.node.upgrade().unwrap().aliases.get_valid_contents() {
			alias.enabled.store(false, Ordering::Release);
		}

		debug_span!("Process input method").in_scope(|| {
			// Get all valid input handlers and convert them to DistanceLink objects
			let distance_links: Vec<DistanceLink> = debug_span!("Generate distance links")
				.in_scope(|| {
					if let Some(handler_order) = &*method.handler_order.lock() {
						handler_order
							.iter()
							.filter_map(Weak::upgrade)
							.filter(|handler| handler.enabled.load(Ordering::Relaxed))
							.map(|handler| DistanceLink::from(method.clone(), handler))
							.collect()
					} else {
						let mut distance_links: Vec<_> = handlers
							.iter()
							.filter(|handler| handler.enabled.load(Ordering::Relaxed))
							.map(|handler| {
								debug_span!("Create distance link").in_scope(|| {
									DistanceLink::from(method.clone(), handler.clone())
								})
							})
							.collect();

						// Sort the distance links by their distance in ascending order
						debug_span!("Sort distance links").in_scope(|| {
							distance_links.sort_unstable_by(|a, b| {
								a.distance.abs().partial_cmp(&b.distance.abs()).unwrap()
							});
						});

						distance_links.truncate(LIMIT);
						distance_links
					}
				});

			let captures = method.captures.take_valid_contents();
			// Iterate over the distance links and send input to them
			for (i, distance_link) in distance_links.into_iter().enumerate() {
				if let Some(method_alias) = distance_link
					.handler
					.method_aliases
					.get(&(Arc::as_ptr(&distance_link.method) as usize))
					.and_then(|a| a.get_aspect::<Alias>().ok())
				{
					method_alias.enabled.store(true, Ordering::Release);
				}
				let captured = captures.contains(&distance_link.handler);
				distance_link.send_input(i as u32, captured, method.datamap.lock().clone());

				// If the current distance link is in the list of captured input handlers,
				// break out of the loop to avoid sending input to the remaining distance links
				if captured {
					break;
				}
			}
		});
	}
}

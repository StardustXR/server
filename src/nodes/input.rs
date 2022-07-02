use super::core::Node;
use super::field::Field;
use super::spatial::{get_spatial_parent_flex, get_transform_pose_flex, Spatial};
use crate::core::client::Client;
use crate::core::eventloop::FRAME;
use crate::core::registry::Registry;
use anyhow::{anyhow, ensure, Result};
use lazy_static::lazy_static;
use std::ops::Deref;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};

lazy_static! {
	static ref INPUT_METHOD_REGISTRY: Registry<InputMethod> = Default::default();
	static ref INPUT_HANDLER_REGISTRY: Registry<InputHandler> = Default::default();
}

pub trait InputSpecializationTrait {
	fn distance(&self, space: &Spatial, field: &Field) -> f32;
	fn serialize(&self, space: &Spatial, distance: f32) -> Vec<u8>;
}
enum InputSpecialization {}
impl Deref for InputSpecialization {
	type Target = dyn InputSpecializationTrait;
	fn deref(&self) -> &Self::Target {
		todo!()
		// match self {
		// 	Field::Box(field) => field,
		// }
	}
}

pub struct InputMethod {
	spatial: Arc<Spatial>,
	specialization: InputSpecialization,
}
impl InputMethod {
	fn add_to(node: &Arc<Node>, specialization: InputSpecialization) -> Result<()> {
		ensure!(
			node.spatial.get().is_some(),
			"Internal: Node does not have a spatial attached!"
		);

		let method = InputMethod {
			spatial: node.spatial.get().unwrap().clone(),
			specialization,
		};
		let method = INPUT_METHOD_REGISTRY.add(method);
		let _ = node.input_method.set(method);
		Ok(())
	}
	fn distance(&self, to: &InputHandler) -> Option<f32> {
		to.field
			.upgrade()
			.map(|field| self.specialization.distance(&self.spatial, &field))
	}
}
impl Drop for InputMethod {
	fn drop(&mut self) {
		INPUT_METHOD_REGISTRY.remove(self);
	}
}

struct DistanceLink {
	distance: f32,
	method: Weak<InputMethod>,
	handler: Weak<InputHandler>,
}
impl DistanceLink {
	fn from(method: &Arc<InputMethod>, handler: &Arc<InputHandler>) -> Option<Self> {
		Some(DistanceLink {
			distance: method.distance(handler)?,
			method: Arc::downgrade(method),
			handler: Arc::downgrade(handler),
		})
	}
	fn serialize(&self) -> Option<Vec<u8>> {
		self.method.upgrade().and_then(|method| {
			self.handler.upgrade().map(|handler| {
				method
					.specialization
					.serialize(&handler.spatial.upgrade().unwrap(), self.distance)
			})
		})
	}
}

pub struct InputHandler {
	node: Weak<Node>,
	spatial: Weak<Spatial>,
	field: Weak<Field>,
}
impl InputHandler {
	pub fn add_to(node: &Arc<Node>, field: &Arc<Field>) -> Result<()> {
		ensure!(
			node.spatial.get().is_some(),
			"Internal: Node does not have a spatial attached!"
		);

		let handler = InputHandler {
			node: Arc::downgrade(node),
			spatial: Arc::downgrade(node.spatial.get().unwrap()),
			field: Arc::downgrade(field),
		};
		let handler = INPUT_HANDLER_REGISTRY.add(handler);
		let _ = node.input_handler.set(handler);
		Ok(())
	}

	fn send_input(
		&self,
		old_frame: u64,
		distance_link: DistanceLink,
		distance_links: Vec<DistanceLink>,
	) {
		if old_frame < FRAME.load(Ordering::Relaxed) {
			return;
		}

		if let Some(data) = distance_link.serialize() {
			let _ = self.node.upgrade().unwrap().execute_remote_method(
				"input",
				&data,
				Box::new(move |data| {
					let capture = flexbuffers::Reader::get_root(data)
						.and_then(|data| data.get_bool())
						.unwrap_or(false);
					if !distance_links.is_empty() && !capture {
						InputHandler::next_input(old_frame, distance_links);
					}
				}),
			);
		} else {
			InputHandler::next_input(old_frame, distance_links);
		}
	}

	fn next_input(old_frame: u64, distance_links: Vec<DistanceLink>) {
		let mut distance_links = distance_links;
		if let Some(distance_link) = distance_links.pop() {
			if let Some(handler) = distance_link.handler.upgrade() {
				handler.send_input(old_frame, distance_link, distance_links);
			}
		}
	}
}
impl Drop for InputHandler {
	fn drop(&mut self) {
		INPUT_HANDLER_REGISTRY.remove(self);
	}
}

pub fn create_interface(client: &Arc<Client>) {
	let node = Node::create(client, "", "data", false);
	node.add_local_signal("createInputHandler", create_input_handler_flex);
	node.add_to_scenegraph();
}

pub fn create_input_handler_flex(
	_node: &Node,
	calling_client: Arc<Client>,
	data: &[u8],
) -> Result<()> {
	let root = flexbuffers::Reader::get_root(data)?;
	let flex_vec = root.get_vector()?;
	let node = Node::create(
		&calling_client,
		"/input/handler",
		flex_vec.idx(0).get_str()?,
		true,
	);
	let parent = get_spatial_parent_flex(&calling_client, flex_vec.idx(1).get_str()?)?;
	let transform = get_transform_pose_flex(&flex_vec.idx(2), &flex_vec.idx(3))?;
	let field = calling_client
		.scenegraph
		.get_node(flex_vec.idx(4).as_str())
		.ok_or_else(|| anyhow!("Field not found"))?
		.field
		.get()
		.ok_or_else(|| anyhow!("Field node is not a field"))?
		.clone();

	let node = node.add_to_scenegraph();
	Spatial::add_to(&node, Some(parent), transform)?;
	InputHandler::add_to(&node, &field)?;
	Ok(())
}

#[allow(dead_code)]
pub fn process_input() {
	for method in INPUT_METHOD_REGISTRY.get_valid_contents() {
		let mut distance_links: Vec<DistanceLink> = Default::default();
		for handler in INPUT_HANDLER_REGISTRY.get_valid_contents() {
			if let Some(distance_link) = DistanceLink::from(&method, &handler) {
				distance_links.push(distance_link);
			}
		}
		if distance_links.is_empty() {
			continue;
		}
		distance_links
			.sort_unstable_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap().reverse());
		InputHandler::next_input(FRAME.load(Ordering::Relaxed), distance_links);
	}
}

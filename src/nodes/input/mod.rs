pub mod pointer;

use self::pointer::Pointer;

use super::fields::Field;
use super::spatial::{get_spatial_parent_flex, get_transform_pose_flex, Spatial};
use super::Node;
use crate::core::client::Client;
use crate::core::eventloop::FRAME;
use crate::core::registry::Registry;
use anyhow::{anyhow, ensure, Result};
use glam::Mat4;
use libstardustxr::schemas::input::{InputData, InputDataArgs, InputDataRaw};
use nanoid::nanoid;
use std::ops::Deref;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};

static INPUT_METHOD_REGISTRY: Registry<InputMethod> = Registry::new();
static INPUT_HANDLER_REGISTRY: Registry<InputHandler> = Registry::new();

pub trait InputSpecialization: Send + Sync {
	fn distance(&self, space: &Arc<Spatial>, field: &Field) -> f32;
	fn serialize(
		&self,
		fbb: &mut flatbuffers::FlatBufferBuilder,
		distance_link: &DistanceLink,
		local_to_handler_matrix: Mat4,
	) -> (
		InputDataRaw,
		flatbuffers::WIPOffset<flatbuffers::UnionWIPOffset>,
	);
	fn serialize_datamap(&self) -> Vec<u8>;
}
pub enum InputType {
	Pointer(Pointer),
}
impl Deref for InputType {
	type Target = dyn InputSpecialization;
	fn deref(&self) -> &Self::Target {
		match self {
			InputType::Pointer(p) => p,
		}
	}
}

pub struct InputMethod {
	pub uid: String,
	pub spatial: Arc<Spatial>,
	pub specialization: InputType,
	pub captures: Registry<InputHandler>,
}
impl InputMethod {
	pub fn new(spatial: Arc<Spatial>, specialization: InputType) -> Arc<InputMethod> {
		let method = InputMethod {
			uid: nanoid!(),
			spatial,
			specialization,
			captures: Registry::new(),
		};
		INPUT_METHOD_REGISTRY.add(method)
	}
	#[allow(dead_code)]
	pub fn add_to(node: &Arc<Node>, specialization: InputType) -> Result<()> {
		ensure!(
			node.spatial.get().is_some(),
			"Internal: Node does not have a spatial attached!"
		);

		let method = InputMethod {
			uid: node.uid.clone(),
			spatial: node.spatial.get().unwrap().clone(),
			specialization,
			captures: Registry::new(),
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
	fn serialize_datamap(&self) -> Vec<u8> {
		self.specialization.serialize_datamap()
	}
}
impl Drop for InputMethod {
	fn drop(&mut self) {
		INPUT_METHOD_REGISTRY.remove(self);
	}
}

pub struct DistanceLink {
	pub distance: f32,
	pub method: Arc<InputMethod>,
	pub handler: Arc<InputHandler>,
}
impl DistanceLink {
	fn from(method: Arc<InputMethod>, handler: Arc<InputHandler>) -> Option<Self> {
		Some(DistanceLink {
			distance: method.distance(&handler)?,
			method,
			handler,
		})
	}

	fn send_input(&self, frame: u64, datamap: &[u8]) {
		self.handler.send_input(frame, self, datamap);
	}
	fn serialize(&self, datamap: &[u8]) -> Vec<u8> {
		let mut fbb = flatbuffers::FlatBufferBuilder::with_capacity(1024);
		let uid = Some(fbb.create_string(&self.method.uid));
		let datamap = Some(fbb.create_vector(datamap));

		let (input_type, input_data) = self.method.specialization.serialize(
			&mut fbb,
			self,
			Spatial::space_to_space_matrix(Some(&self.method.spatial), Some(&self.handler.spatial)),
		);

		let root = InputData::create(
			&mut fbb,
			&InputDataArgs {
				uid,
				input_type,
				input: Some(input_data),
				distance: self.distance,
				datamap,
			},
		);
		fbb.finish(root, None);
		Vec::from(fbb.finished_data())
	}
}

pub struct InputHandler {
	node: Weak<Node>,
	spatial: Arc<Spatial>,
	pub field: Weak<Field>,
}
impl InputHandler {
	pub fn add_to(node: &Arc<Node>, field: &Arc<Field>) -> Result<()> {
		ensure!(
			node.spatial.get().is_some(),
			"Internal: Node does not have a spatial attached!"
		);

		let handler = InputHandler {
			node: Arc::downgrade(node),
			spatial: node.spatial.get().unwrap().clone(),
			field: Arc::downgrade(field),
		};
		let handler = INPUT_HANDLER_REGISTRY.add(handler);
		let _ = node.input_handler.set(handler);
		Ok(())
	}

	fn send_input(&self, frame: u64, distance_link: &DistanceLink, datamap: &[u8]) {
		let data = distance_link.serialize(datamap);
		let node = self.node.upgrade().unwrap();
		let method = Arc::downgrade(&distance_link.method);
		let handler = Arc::downgrade(&distance_link.handler);

		tokio::spawn(async move {
			let data = node.execute_remote_method("input", data).await;
			if frame == FRAME.load(Ordering::Relaxed) {
				if let Ok(data) = data {
					let capture = flexbuffers::Reader::get_root(data.as_slice())
						.and_then(|data| data.get_bool())
						.unwrap_or(false);

					if let Some(method) = method.upgrade() {
						if let Some(handler) = handler.upgrade() {
							if capture {
								method.captures.add_raw(&handler);
							}
						}
					}
				}
			}
		});
	}
}
impl Drop for InputHandler {
	fn drop(&mut self) {
		INPUT_HANDLER_REGISTRY.remove(self);
	}
}

pub fn create_interface(client: &Arc<Client>) {
	let node = Node::create(client, "", "input", false);
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

pub fn process_input() {
	for method in INPUT_METHOD_REGISTRY.get_valid_contents() {
		let mut distance_links: Vec<DistanceLink> = INPUT_HANDLER_REGISTRY
			.get_valid_contents()
			.into_iter()
			.filter_map(|handler| DistanceLink::from(method.clone(), handler))
			.collect();
		distance_links
			.sort_unstable_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap().reverse());

		let datamap = method.serialize_datamap();
		let frame = FRAME.load(Ordering::Relaxed);
		let captures = method.captures.get_valid_contents();
		for distance_link in distance_links {
			distance_link.send_input(frame, &datamap);
			if captures
				.iter()
				.any(|c| Arc::ptr_eq(c, &distance_link.handler))
			{
				break;
			}
		}
		method.captures.clear();
	}
}

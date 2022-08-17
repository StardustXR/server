use super::core::Node;
use super::field::Field;
use super::spatial::{get_spatial_parent_flex, get_transform_pose_flex, Spatial};
use crate::core::client::Client;
use crate::core::eventloop::FRAME;
use crate::core::registry::Registry;
use anyhow::{anyhow, ensure, Result};
use glam::Mat4;
use libstardustxr::schemas::input::{InputData, InputDataArgs, InputDataRaw};
use std::ops::Deref;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};

static INPUT_METHOD_REGISTRY: Registry<InputMethod> = Registry::new();
static INPUT_HANDLER_REGISTRY: Registry<InputHandler> = Registry::new();

pub trait InputSpecializationTrait {
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
	uid: String,
	pub spatial: Arc<Spatial>,
	specialization: InputSpecialization,
}
impl InputMethod {
	fn add_to(node: &Arc<Node>, specialization: InputSpecialization) -> Result<()> {
		ensure!(
			node.spatial.get().is_some(),
			"Internal: Node does not have a spatial attached!"
		);

		let method = InputMethod {
			uid: node.uid.clone(),
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

pub struct DistanceLink {
	pub distance: f32,
	pub method: Weak<InputMethod>,
	pub handler: Weak<InputHandler>,
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
				let mut fbb = flatbuffers::FlatBufferBuilder::with_capacity(1024);
				let uid = Some(fbb.create_string(&method.uid));
				let datamap = Some(fbb.create_vector(&self.serialize_datamap()));

				let (input_type, input_data) = method.specialization.serialize(
					&mut fbb,
					self,
					Spatial::space_to_space_matrix(Some(&method.spatial), Some(&handler.spatial)),
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
			})
		})
	}
	fn serialize_datamap(&self) -> Vec<u8> {
		if let Some(method) = self.method.upgrade() {
			method.specialization.serialize_datamap()
		} else {
			Default::default()
		}
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

	fn send_input(
		&self,
		old_frame: u64,
		distance_link: DistanceLink,
		distance_links: Vec<DistanceLink>,
	) {
		if old_frame < FRAME.load(Ordering::Relaxed) {
			return;
		}

		match distance_link.serialize() {
			None => InputHandler::next_input(old_frame, distance_links),
			Some(data) => {
				let node = self.node.upgrade().unwrap();

				tokio::spawn(async move {
					let data = node.execute_remote_method("input", data).await;
					if let Ok(data) = data {
						let capture = flexbuffers::Reader::get_root(data.as_slice())
							.and_then(|data| data.get_bool())
							.unwrap_or(false);
						if !distance_links.is_empty() && !capture {
							InputHandler::next_input(old_frame, distance_links);
						}
					}
				});
			}
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

pub mod hand;
pub mod pointer;
pub mod tip;

use self::hand::Hand;
use self::pointer::Pointer;
use self::tip::Tip;

use super::{
	alias::{Alias, AliasInfo},
	fields::{find_field, Field},
	spatial::{find_spatial_parent, parse_transform, Spatial},
	Node,
};
use crate::core::client::Client;
use crate::core::eventloop::FRAME;
use crate::core::{node_collections::LifeLinkedNodeList, registry::Registry};
use color_eyre::eyre::{ensure, Result};
use glam::Mat4;
use parking_lot::Mutex;
use portable_atomic::AtomicBool;
use serde::Deserialize;
use stardust_xr::schemas::flat::{Datamap, InputDataType};
use stardust_xr::schemas::{flat::InputData, flex::deserialize};
use stardust_xr::values::Transform;
use std::ops::Deref;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};
use tracing::{debug_span, instrument};

static INPUT_METHOD_REGISTRY: Registry<InputMethod> = Registry::new();
static INPUT_HANDLER_REGISTRY: Registry<InputHandler> = Registry::new();

pub trait InputSpecialization: Send + Sync {
	fn compare_distance(&self, space: &Arc<Spatial>, field: &Field) -> f32;
	fn true_distance(&self, space: &Arc<Spatial>, field: &Field) -> f32;
	fn serialize(
		&self,
		distance_link: &DistanceLink,
		local_to_handler_matrix: Mat4,
	) -> InputDataType;
}
pub enum InputType {
	Pointer(Pointer),
	Hand(Box<Hand>),
	Tip(Tip),
}
impl Deref for InputType {
	type Target = dyn InputSpecialization;
	fn deref(&self) -> &Self::Target {
		match self {
			InputType::Pointer(p) => p,
			InputType::Hand(h) => h.as_ref(),
			InputType::Tip(t) => t,
		}
	}
}

pub struct InputMethod {
	node: Weak<Node>,
	uid: String,
	pub enabled: Mutex<bool>,
	pub spatial: Arc<Spatial>,
	pub specialization: Mutex<InputType>,
	captures: Registry<InputHandler>,
	pub datamap: Mutex<Option<Datamap>>,
}
impl InputMethod {
	#[allow(dead_code)]
	pub fn add_to(
		node: &Arc<Node>,
		specialization: InputType,
		datamap: Option<Datamap>,
	) -> Result<Arc<InputMethod>> {
		ensure!(
			node.spatial.get().is_some(),
			"Internal: Node does not have a spatial attached!"
		);

		node.add_local_signal("capture", InputMethod::capture_flex);
		node.add_local_signal("set_datamap", InputMethod::set_datamap_flex);

		let method = InputMethod {
			node: Arc::downgrade(node),
			uid: node.uid.clone(),
			enabled: Mutex::new(true),
			spatial: node.spatial.get().unwrap().clone(),
			specialization: Mutex::new(specialization),
			captures: Registry::new(),
			datamap: Mutex::new(datamap),
		};
		let method = INPUT_METHOD_REGISTRY.add(method);
		let _ = node.input_method.set(method.clone());
		Ok(method)
	}

	fn capture_flex(node: &Node, calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		let method = node.get_aspect("Input Method", "input method", |n| &n.input_method)?;
		let handler_node = calling_client.get_node("Input Handler", deserialize(data)?)?;
		let handler =
			handler_node.get_aspect("Input Handler", "input handler", |n| &n.input_handler)?;

		method.captures.add_raw(&handler);
		Ok(())
	}
	fn set_datamap_flex(node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		let method = node.get_aspect("Input Method", "input method", |n| &n.input_method)?;
		method.datamap.lock().replace(Datamap::new(data.to_vec())?);
		Ok(())
	}

	fn compare_distance(&self, to: &Field) -> f32 {
		self.specialization
			.lock()
			.compare_distance(&self.spatial, to)
	}
	fn true_distance(&self, to: &Field) -> f32 {
		self.specialization.lock().true_distance(&self.spatial, to)
	}
}
impl Drop for InputMethod {
	fn drop(&mut self) {
		INPUT_METHOD_REGISTRY.remove(self);
	}
}

pub struct DistanceLink {
	distance: f32,
	method: Arc<InputMethod>,
	handler: Arc<InputHandler>,
}
impl DistanceLink {
	fn from(method: Arc<InputMethod>, handler: Arc<InputHandler>) -> Option<Self> {
		let handler_node = handler.node.upgrade()?;
		let method_alias = Alias::create(
			&handler_node.get_client()?,
			handler_node.get_path(),
			&method.uid,
			&method.node.upgrade()?,
			AliasInfo {
				local_signals: vec!["capture"],
				..Default::default()
			},
		)
		.ok()?;
		handler.method_aliases.add(Arc::downgrade(&method_alias));
		Some(DistanceLink {
			distance: method.compare_distance(&handler.field),
			method,
			handler,
		})
	}

	fn send_input(&self, frame: u64, datamap: Datamap) {
		self.handler.send_input(frame, self, datamap);
	}
	#[instrument(level = "debug", skip(self))]
	fn serialize(&self, datamap: Datamap) -> Vec<u8> {
		let input = self.method.specialization.lock().serialize(
			self,
			Spatial::space_to_space_matrix(Some(&self.method.spatial), Some(&self.handler.spatial)),
		);

		let root = InputData {
			uid: self.method.uid.clone(),
			input,
			distance: self.method.true_distance(&self.handler.field),
			datamap,
		};
		root.serialize()
	}
}

pub struct InputHandler {
	enabled: Arc<AtomicBool>,
	node: Weak<Node>,
	spatial: Arc<Spatial>,
	field: Arc<Field>,
	method_aliases: LifeLinkedNodeList,
}
impl InputHandler {
	pub fn add_to(node: &Arc<Node>, field: &Arc<Field>) -> Result<()> {
		ensure!(
			node.spatial.get().is_some(),
			"Internal: Node does not have a spatial attached!"
		);

		let handler = InputHandler {
			enabled: node.enabled.clone(),
			node: Arc::downgrade(node),
			spatial: node.spatial.get().unwrap().clone(),
			field: field.clone(),
			method_aliases: LifeLinkedNodeList::default(),
		};
		let handler = INPUT_HANDLER_REGISTRY.add(handler);
		let _ = node.input_handler.set(handler);
		Ok(())
	}

	#[instrument(level = "debug", skip(self, distance_link))]
	fn send_input(&self, frame: u64, distance_link: &DistanceLink, datamap: Datamap) {
		let Some(node) = self.node.upgrade() else {return};
		let _ = node.send_remote_signal("input", &distance_link.serialize(datamap));
	}
}
impl PartialEq for InputHandler {
	fn eq(&self, other: &Self) -> bool {
		self.spatial == other.spatial
	}
}
impl Drop for InputHandler {
	fn drop(&mut self) {
		INPUT_HANDLER_REGISTRY.remove(self);
	}
}

pub fn create_interface(client: &Arc<Client>) -> Result<()> {
	let node = Node::create(client, "", "input", false);
	node.add_local_signal("create_input_handler", create_input_handler_flex);
	node.add_local_signal("create_input_method_tip", tip::create_tip_flex);
	node.add_to_scenegraph().map(|_| ())
}

pub fn create_input_handler_flex(
	_node: &Node,
	calling_client: Arc<Client>,
	data: &[u8],
) -> Result<()> {
	#[derive(Deserialize)]
	struct CreateInputHandlerInfo<'a> {
		name: &'a str,
		parent_path: &'a str,
		transform: Transform,
		field_path: &'a str,
	}
	let info: CreateInputHandlerInfo = deserialize(data)?;
	let parent = find_spatial_parent(&calling_client, info.parent_path)?;
	let transform = parse_transform(info.transform, true, true, true);
	let field = find_field(&calling_client, info.field_path)?;

	let node =
		Node::create(&calling_client, "/input/handler", info.name, true).add_to_scenegraph()?;
	Spatial::add_to(&node, Some(parent), transform, false)?;
	InputHandler::add_to(&node, &field)?;
	Ok(())
}
#[tracing::instrument(level = "debug")]
pub fn process_input() {
	// Iterate over all valid input methods
	let methods = debug_span!("Get valid methods").in_scope(|| {
		INPUT_METHOD_REGISTRY
			.get_valid_contents()
			.into_iter()
			.filter(|method| *method.enabled.lock())
			.filter(|method| method.datamap.lock().is_some())
	});
	let handlers = INPUT_HANDLER_REGISTRY.get_valid_contents();
	for handler in &handlers {
		handler.method_aliases.clear();
	}
	for method in methods {
		debug_span!("Process input method").in_scope(|| {
			// Get all valid input handlers and convert them to DistanceLink objects
			let mut distance_links: Vec<DistanceLink> = debug_span!("Generate distance links")
				.in_scope(|| {
					handlers
						.iter()
						.filter(|handler| handler.enabled.load(Ordering::Relaxed))
						.filter_map(|handler| DistanceLink::from(method.clone(), handler.clone()))
						.collect()
				});

			// Sort the distance links by their distance in ascending order
			debug_span!("Sort distance links").in_scope(|| {
				distance_links.sort_unstable_by(|a, b| {
					a.distance.abs().partial_cmp(&b.distance.abs()).unwrap()
				});
			});

			// Get the current frame
			let frame = FRAME.load(Ordering::Relaxed);

			let captures = method.captures.take_valid_contents();
			// Iterate over the distance links and send input to them
			for distance_link in distance_links {
				distance_link.send_input(frame, method.datamap.lock().clone().unwrap());

				// If the current distance link is in the list of captured input handlers,
				// break out of the loop to avoid sending input to the remaining distance links
				if captures.contains(&distance_link.handler) {
					break;
				}
			}
		});
	}
}

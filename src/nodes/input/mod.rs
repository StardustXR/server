pub mod hand;
pub mod pointer;
pub mod tip;

use self::hand::Hand;
use self::pointer::Pointer;
use self::tip::Tip;

use super::{
	alias::{Alias, AliasInfo},
	fields::{find_field, Field, FIELD_ALIAS_INFO},
	spatial::{find_spatial_parent, parse_transform, Spatial},
	Message, Node,
};
use crate::core::registry::Registry;
use crate::core::{client::Client, node_collections::LifeLinkedNodeMap};
use color_eyre::eyre::{ensure, Result};
use glam::Mat4;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use portable_atomic::AtomicBool;
use serde::Deserialize;
use stardust_xr::schemas::{flat::InputData, flex::deserialize};
use stardust_xr::schemas::{
	flat::{Datamap, InputDataType},
	flex::serialize,
};
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
	handler_aliases: LifeLinkedNodeMap<String>,
	handler_order: OnceCell<Mutex<Vec<Weak<InputHandler>>>>,
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
		node.add_local_signal("set_handlers", InputMethod::set_handlers_flex);

		let method = InputMethod {
			node: Arc::downgrade(node),
			uid: node.uid.clone(),
			enabled: Mutex::new(true),
			spatial: node.spatial.get().unwrap().clone(),
			specialization: Mutex::new(specialization),
			captures: Registry::new(),
			datamap: Mutex::new(datamap),
			handler_aliases: LifeLinkedNodeMap::default(),
			handler_order: OnceCell::new(),
		};
		for handler in INPUT_HANDLER_REGISTRY.get_valid_contents() {
			method.handle_new_handler(&handler);
			method.make_alias(&handler);
		}
		let method = INPUT_METHOD_REGISTRY.add(method);
		let _ = node.input_method.set(method.clone());
		Ok(method)
	}
	fn get(node: &Node) -> Result<Arc<Self>> {
		node.get_aspect("Input Method", "input method", |n| &n.input_method)
			.cloned()
	}

	fn capture_flex(node: &Node, calling_client: Arc<Client>, message: Message) -> Result<()> {
		let method = InputMethod::get(node)?;
		let handler = InputHandler::find(&calling_client, deserialize(message.as_ref())?)?;

		method.captures.add_raw(&handler);
		node.send_remote_signal("capture", message)
	}
	fn set_datamap_flex(node: &Node, _calling_client: Arc<Client>, message: Message) -> Result<()> {
		let method = InputMethod::get(node)?;
		method.datamap.lock().replace(Datamap::new(message.data)?);
		Ok(())
	}
	fn set_handlers_flex(node: &Node, calling_client: Arc<Client>, message: Message) -> Result<()> {
		let method = InputMethod::get(node)?;
		let handler_paths: Vec<&str> = deserialize(message.as_ref())?;
		let handlers: Vec<Weak<InputHandler>> = handler_paths
			.into_iter()
			.filter_map(|p| InputHandler::find(&calling_client, p).ok())
			.map(|h| Arc::downgrade(&h))
			.collect();

		*method
			.handler_order
			.get_or_init(|| Mutex::new(Vec::new()))
			.lock() = handlers;
		Ok(())
	}

	fn make_alias(&self, handler: &InputHandler) {
		let Some(method_node) = self.node.upgrade() else {return};
		let Some(handler_node) = handler.node.upgrade() else {return};
		let Some(client) = handler_node.get_client() else {return};
		let Ok(method_alias) = Alias::create(
			&client,
			handler_node.get_path(),
			&self.uid,
			&method_node,
			AliasInfo {
				server_signals: vec!["capture"],
				..Default::default()
			},
		) else {return};
		method_alias.enabled.store(false, Ordering::Relaxed);
		handler
			.method_aliases
			.add(self as *const InputMethod as usize, &method_alias);
	}

	fn compare_distance(&self, to: &Field) -> f32 {
		self.specialization
			.lock()
			.compare_distance(&self.spatial, to)
	}
	fn true_distance(&self, to: &Field) -> f32 {
		self.specialization.lock().true_distance(&self.spatial, to)
	}

	fn handle_new_handler(&self, handler: &InputHandler) {
		let Some(method_node) = self.node.upgrade() else {return};
		let Some(method_client) = method_node.get_client() else {return};
		let Some(handler_node) = handler.node.upgrade() else {return};
		// Receiver itself
		let Ok(handler_alias) = Alias::create(
			&method_client,
			method_node.get_path(),
			handler.uid.as_str(),
			&handler_node,
			AliasInfo {
				server_methods: vec!["getTransform"],
				..Default::default()
			},
		) else {return};
		self.handler_aliases
			.add(handler.uid.clone(), &handler_alias);

		if let Some(handler_field_node) = handler.field.spatial_ref().node.upgrade() {
			// Handler's field
			let Ok(rx_field_alias) = Alias::create(
					&method_client,
					handler_alias.get_path(),
					"field",
					&handler_field_node,
					FIELD_ALIAS_INFO.clone(),
				) else {return};
			self.handler_aliases
				.add(handler.uid.clone() + "-field", &rx_field_alias);
		}

		let Ok(data) = serialize(&handler.uid) else {return};
		let _ = method_node.send_remote_signal("handler_created", data);
	}
	fn handle_drop_handler(&self, handler: &InputHandler) {
		let uid = handler.uid.as_str();
		self.handler_aliases.remove(uid);
		self.handler_aliases.remove(&(uid.to_string() + "-field"));
		let Some(tx_node) = self.node.upgrade() else {return};
		let Ok(data) = serialize(&uid) else {return};
		let _ = tx_node.send_remote_signal("handler_destroyed", data);
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
	fn from(method: Arc<InputMethod>, handler: Arc<InputHandler>) -> Self {
		DistanceLink {
			distance: method.compare_distance(&handler.field),
			method,
			handler,
		}
	}

	fn send_input(&self, order: u32, captured: bool, datamap: Datamap) {
		self.handler.send_input(order, captured, self, datamap);
	}
	#[instrument(level = "debug", skip(self))]
	fn serialize(&self, order: u32, captured: bool, datamap: Datamap) -> Vec<u8> {
		let input = self.method.specialization.lock().serialize(
			self,
			Spatial::space_to_space_matrix(Some(&self.method.spatial), Some(&self.handler.spatial)),
		);

		let root = InputData {
			uid: self.method.uid.clone(),
			input,
			distance: self.method.true_distance(&self.handler.field),
			datamap,
			order,
			captured,
		};
		root.serialize()
	}
}

pub struct InputHandler {
	enabled: Arc<AtomicBool>,
	uid: String,
	node: Weak<Node>,
	spatial: Arc<Spatial>,
	field: Arc<Field>,
	method_aliases: LifeLinkedNodeMap<usize>,
}
impl InputHandler {
	pub fn add_to(node: &Arc<Node>, field: &Arc<Field>) -> Result<()> {
		ensure!(
			node.spatial.get().is_some(),
			"Internal: Node does not have a spatial attached!"
		);

		let handler = InputHandler {
			enabled: node.enabled.clone(),
			uid: node.uid.clone(),
			node: Arc::downgrade(node),
			spatial: node.spatial.get().unwrap().clone(),
			field: field.clone(),
			method_aliases: LifeLinkedNodeMap::default(),
		};
		for method in INPUT_METHOD_REGISTRY.get_valid_contents() {
			method.make_alias(&handler);
			method.handle_new_handler(&handler);
		}
		let handler = INPUT_HANDLER_REGISTRY.add(handler);
		let _ = node.input_handler.set(handler);
		Ok(())
	}
	fn find(client: &Client, path: &str) -> Result<Arc<Self>> {
		InputHandler::get(&*client.get_node("Input Handler", path)?)
	}
	fn get(node: &Node) -> Result<Arc<Self>> {
		node.get_aspect("Input Handler", "input handler", |n| &n.input_handler)
			.cloned()
	}

	#[instrument(level = "debug", skip(self, distance_link))]
	fn send_input(
		&self,
		order: u32,
		captured: bool,
		distance_link: &DistanceLink,
		datamap: Datamap,
	) {
		let Some(node) = self.node.upgrade() else {return};
		let _ = node.send_remote_signal("input", distance_link.serialize(order, captured, datamap));
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
		for method in INPUT_METHOD_REGISTRY.get_valid_contents() {
			method.handle_drop_handler(self);
		}
	}
}

pub fn create_interface(client: &Arc<Client>) -> Result<()> {
	let node = Node::create(client, "", "input", false);
	node.add_local_signal("create_input_handler", create_input_handler_flex);
	node.add_local_signal("create_input_method_pointer", pointer::create_pointer_flex);
	node.add_local_signal("create_input_method_tip", tip::create_tip_flex);
	node.add_to_scenegraph().map(|_| ())
}

pub fn create_input_handler_flex(
	_node: &Node,
	calling_client: Arc<Client>,
	message: Message,
) -> Result<()> {
	#[derive(Deserialize)]
	struct CreateInputHandlerInfo<'a> {
		name: &'a str,
		parent_path: &'a str,
		transform: Transform,
		field_path: &'a str,
	}
	let info: CreateInputHandlerInfo = deserialize(message.as_ref())?;
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
	const LIMIT: usize = 50;
	for method in methods {
		for alias in method.node.upgrade().unwrap().aliases.get_valid_contents() {
			alias.enabled.store(false, Ordering::Release);
		}

		debug_span!("Process input method").in_scope(|| {
			// Get all valid input handlers and convert them to DistanceLink objects
			let distance_links: Vec<DistanceLink> = debug_span!("Generate distance links")
				.in_scope(|| {
					if let Some(handler_order) = method.handler_order.get() {
						let handler_order = handler_order.lock();
						handler_order
							.iter()
							.filter_map(|h| h.upgrade())
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
				if i > LIMIT {
					break;
				}

				if let Some(method_alias) = distance_link
					.handler
					.method_aliases
					.get(&(Arc::as_ptr(&distance_link.method) as usize))
					.and_then(|a| a.alias.get().cloned())
				{
					method_alias.enabled.store(true, Ordering::Release);
				}
				let captured = captures.contains(&distance_link.handler);
				distance_link.send_input(
					i as u32,
					captured,
					method.datamap.lock().clone().unwrap(),
				);

				// If the current distance link is in the list of captured input handlers,
				// break out of the loop to avoid sending input to the remaining distance links
				if captured {
					break;
				}
			}
		});
	}
}

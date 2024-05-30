pub mod camera;
pub mod panel;

use self::camera::CameraItem;
use self::panel::PanelItemTrait;
use super::fields::Field;
use super::spatial::Spatial;
use super::{Alias, Aspect, Message, Node};
use crate::core::client::Client;
use crate::core::node_collections::LifeLinkedNodeMap;
use crate::core::registry::Registry;
use crate::nodes::alias::AliasInfo;
use crate::nodes::spatial::Transform;
use color_eyre::eyre::{ensure, Result};
use lazy_static::lazy_static;
use nanoid::nanoid;
use parking_lot::Mutex;
use portable_atomic::Ordering;
use stardust_xr::schemas::flex::deserialize;

use std::hash::Hash;
use std::sync::{Arc, Weak};

stardust_xr_server_codegen::codegen_item_protocol!();

lazy_static! {
	static ref ITEM_ALIAS_LOCAL_SIGNALS: Vec<&'static str> = vec![
		"get_bounds",
		"get_transform",
		"set_transform",
		"set_spatial_parent",
		"set_spatial_parent_in_place",
		"set_zoneable",
		"release",
	];
	static ref ITEM_ALIAS_LOCAL_METHODS: Vec<&'static str> = vec![];
	static ref ITEM_ALIAS_REMOTE_SIGNALS: Vec<&'static str> = vec![];
}

pub fn capture(item: &Arc<Item>, acceptor: &Arc<ItemAcceptor>) {
	if item.captured_acceptor.lock().strong_count() > 0 {
		release(item);
	}
	*item.captured_acceptor.lock() = Arc::downgrade(acceptor);
	acceptor.handle_capture(item);
	if let Some(ui) = item.type_info.ui.lock().upgrade() {
		ui.handle_capture_item(item, acceptor);
	}
}
fn release(item: &Item) {
	let mut captured_acceptor = item.captured_acceptor.lock();
	if let Some(acceptor) = captured_acceptor.upgrade().as_ref() {
		*captured_acceptor = Weak::default();
		acceptor.handle_release(item);
		if let Some(ui) = item.type_info.ui.lock().upgrade() {
			ui.handle_release_item(item, &acceptor);
		}
	}
}

pub struct TypeInfo {
	pub type_name: &'static str,
	pub aliased_local_signals: Vec<&'static str>,
	pub aliased_local_methods: Vec<&'static str>,
	pub aliased_remote_signals: Vec<&'static str>,
	pub ui: Mutex<Weak<ItemUI>>,
	pub items: Registry<Item>,
	pub acceptors: Registry<ItemAcceptor>,
	pub new_acceptor_fn:
		fn(node: &Node, uid: &str, acceptor: &Arc<Node>, acceptor_field: &Arc<Node>),
}
impl Hash for TypeInfo {
	fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
		self.type_name.hash(state);
	}
}
impl PartialEq for TypeInfo {
	fn eq(&self, other: &Self) -> bool {
		self.type_name == other.type_name
	}
}
impl Eq for TypeInfo {}

pub struct Item {
	node: Weak<Node>,
	uid: String,
	type_info: &'static TypeInfo,
	captured_acceptor: Mutex<Weak<ItemAcceptor>>,
	pub specialization: ItemType,
}
impl Item {
	pub fn add_to(
		node: &Arc<Node>,
		uid: String,
		type_info: &'static TypeInfo,
		specialization: ItemType,
	) -> Arc<Self> {
		let item = Item {
			node: Arc::downgrade(node),
			uid,
			type_info,
			captured_acceptor: Default::default(),
			specialization,
		};
		let item = type_info.items.add(item);

		node.add_local_signal("release", Item::release_flex);
		if let Some(ui) = type_info.ui.lock().upgrade() {
			ui.handle_create_item(&item);
		}
		node.add_aspect_raw(item.clone());

		// if let Some(auto_acceptor) = node.get_client().and_then(|client| {
		// 	client
		// 		.state
		// 		.as_ref()
		// 		.and_then(|settings| settings.acceptors.get(type_info))
		// 		.and_then(|acceptor| acceptor.upgrade())
		// }) {
		// 	capture(&item, &auto_acceptor);
		// }

		item
	}
	fn make_alias_named(
		&self,
		client: &Arc<Client>,
		parent: &str,
		name: &str,
	) -> Result<Arc<Node>> {
		Alias::create(
			client,
			parent,
			name,
			&self.node.upgrade().unwrap(),
			AliasInfo {
				server_signals: [
					&self.type_info.aliased_local_signals,
					ITEM_ALIAS_LOCAL_SIGNALS.as_slice(),
				]
				.concat(),
				server_methods: [
					&self.type_info.aliased_local_methods,
					ITEM_ALIAS_LOCAL_METHODS.as_slice(),
				]
				.concat(),
				client_signals: [
					&self.type_info.aliased_remote_signals,
					ITEM_ALIAS_REMOTE_SIGNALS.as_slice(),
				]
				.concat(),
			},
		)
	}
	fn make_alias(&self, client: &Arc<Client>, parent: &str) -> Result<Arc<Node>> {
		self.make_alias_named(client, parent, &self.uid)
	}

	fn release_flex(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		_message: Message,
	) -> Result<()> {
		let item = node.get_aspect::<Item>()?;
		release(&item);

		Ok(())
	}
}
impl Aspect for Item {
	const NAME: &'static str = "Item";
}
impl Drop for Item {
	fn drop(&mut self) {
		self.type_info.items.remove(self);
		release(self);
		if let Some(ui) = self.type_info.ui.lock().upgrade() {
			ui.handle_destroy_item(self);
		}
	}
}

pub enum ItemType {
	Camera(CameraItem),
	Panel(Arc<dyn PanelItemTrait>),
}
impl ItemType {
	fn send_ui_item_created(&self, node: &Node, uid: &str, item: &Arc<Node>) {
		match self {
			ItemType::Camera(c) => c.send_ui_item_created(node, uid, item),
			ItemType::Panel(p) => p.send_ui_item_created(node, uid, item),
		}
	}
	fn send_acceptor_item_created(&self, node: &Node, uid: &str, item: &Arc<Node>) {
		match self {
			ItemType::Camera(c) => c.send_acceptor_item_created(node, uid, item),
			ItemType::Panel(p) => p.send_acceptor_item_created(node, uid, item),
		}
	}
}
// impl Deref for ItemType {
// 	type Target = dyn ItemSpecialization;

// 	fn deref(&self) -> &Self::Target {
// 		match self {
// 			ItemType::Environment(item) => item,
// 			ItemType::Panel(item) => item.as_ref(),
// 		}
// 	}
// }

pub struct ItemUI {
	node: Weak<Node>,
	type_info: &'static TypeInfo,
	item_aliases: LifeLinkedNodeMap<String>,
	acceptor_aliases: LifeLinkedNodeMap<String>,
	acceptor_field_aliases: LifeLinkedNodeMap<String>,
}
impl ItemUI {
	fn add_to(node: &Arc<Node>, type_info: &'static TypeInfo) -> Result<()> {
		ensure!(
			type_info.ui.lock().upgrade().is_none(),
			"A UI is already active for this type of item"
		);

		let ui = Arc::new(ItemUI {
			node: Arc::downgrade(node),
			type_info,
			item_aliases: Default::default(),
			acceptor_aliases: Default::default(),
			acceptor_field_aliases: Default::default(),
		});
		*type_info.ui.lock() = Arc::downgrade(&ui);
		node.add_aspect_raw(ui.clone());

		for item in type_info.items.get_valid_contents() {
			ui.handle_create_item(&item);
		}
		for acceptor in type_info.acceptors.get_valid_contents() {
			ui.handle_create_acceptor(&acceptor);
		}
		Ok(())
	}

	fn handle_create_item(&self, item: &Item) {
		let Some(node) = self.node.upgrade() else {
			return;
		};
		let Some(client) = node.get_client() else {
			return;
		};

		let Ok(item_alias) = item.make_alias(&client, &(node.get_path().to_string() + "/item"))
		else {
			return;
		};
		self.item_aliases.add(item.uid.clone(), &item_alias);

		item.specialization
			.send_ui_item_created(&node, &item.uid, &item_alias);
	}
	fn handle_capture_item(&self, item: &Item, acceptor: &ItemAcceptor) {
		let _ =
			item_ui_client::capture_item(&self.node.upgrade().unwrap(), &item.uid, &acceptor.uid);
	}
	fn handle_release_item(&self, item: &Item, acceptor: &ItemAcceptor) {
		let _ =
			item_ui_client::release_item(&self.node.upgrade().unwrap(), &item.uid, &acceptor.uid);
	}
	fn handle_destroy_item(&self, item: &Item) {
		let _ = item_ui_client::destroy_item(&self.node.upgrade().unwrap(), &item.uid);
		self.item_aliases.remove(&item.uid);
	}
	fn handle_create_acceptor(&self, acceptor: &ItemAcceptor) {
		let Some(node) = self.node.upgrade() else {
			return;
		};
		let Some(client) = node.get_client() else {
			return;
		};

		let Ok((acceptor_alias, acceptor_field_alias)) = acceptor.make_aliases(
			&client,
			&format!("/item/{}/acceptor", self.type_info.type_name),
		) else {
			return;
		};
		self.acceptor_aliases
			.add(acceptor.uid.clone(), &acceptor_alias);
		self.acceptor_field_aliases
			.add(acceptor.uid.clone(), &acceptor_field_alias);

		(acceptor.type_info.new_acceptor_fn)(
			&node,
			&acceptor.uid,
			&acceptor_alias,
			&acceptor_field_alias,
		);
	}
	fn handle_destroy_acceptor(&self, acceptor: &ItemAcceptor) {
		let _ = item_ui_client::destroy_acceptor(&self.node.upgrade().unwrap(), &acceptor.uid);
		self.acceptor_aliases.remove(&acceptor.uid);
		self.acceptor_field_aliases.remove(&acceptor.uid);
	}
}
impl Aspect for ItemUI {
	const NAME: &'static str = "Item";
}
impl Drop for ItemUI {
	fn drop(&mut self) {
		*self.type_info.ui.lock() = Weak::new();
	}
}

pub struct ItemAcceptor {
	uid: String,
	node: Weak<Node>,
	pub type_info: &'static TypeInfo,
	field: Arc<Field>,
	accepted_aliases: LifeLinkedNodeMap<String>,
	accepted_registry: Registry<Item>,
}
impl ItemAcceptor {
	fn add_to(node: &Arc<Node>, type_info: &'static TypeInfo, field: Arc<Field>) {
		let acceptor = type_info.acceptors.add(ItemAcceptor {
			uid: nanoid!(),
			node: Arc::downgrade(node),
			type_info,
			field,
			accepted_aliases: Default::default(),
			accepted_registry: Registry::new(),
		});
		node.add_local_signal("capture", ItemAcceptor::capture_flex);
		if let Some(ui) = type_info.ui.lock().upgrade() {
			ui.handle_create_acceptor(&acceptor);
		}
		node.add_aspect_raw(acceptor);
	}

	fn capture_flex(node: Arc<Node>, calling_client: Arc<Client>, message: Message) -> Result<()> {
		if !node.enabled.load(Ordering::Relaxed) {
			return Ok(());
		}

		let acceptor = node.get_aspect::<ItemAcceptor>().unwrap();
		let item_path: &str = deserialize(message.as_ref())?;
		let item_node = calling_client.get_node("Item", item_path)?;
		let item = item_node.get_aspect::<Item>()?;
		capture(&item, &acceptor);

		Ok(())
	}

	fn make_aliases(&self, client: &Arc<Client>, parent: &str) -> Result<(Arc<Node>, Arc<Node>)> {
		let acceptor_node = &self.node.upgrade().unwrap();
		let acceptor_alias = Alias::create(
			client,
			parent,
			&self.uid,
			acceptor_node,
			AliasInfo {
				server_signals: vec!["capture"],
				..Default::default()
			},
		)?;

		let acceptor_field_alias = Alias::create(
			client,
			acceptor_alias.get_path(),
			"field",
			&self.field.spatial_ref().node.upgrade().unwrap(),
			AliasInfo::default(),
		)?;

		Ok((acceptor_alias, acceptor_field_alias))
	}
	fn handle_capture(&self, item: &Arc<Item>) {
		let Some(node) = self.node.upgrade() else {
			return;
		};
		let Some(client) = node.get_client() else {
			return;
		};

		self.accepted_registry.add_raw(item);
		let Ok(alias_node) = item.make_alias(&client, &node.path) else {
			return;
		};
		self.accepted_aliases.add(item.uid.clone(), &alias_node);

		item.specialization
			.send_acceptor_item_created(&node, &item.uid, &alias_node);
	}
	fn handle_release(&self, item: &Item) {
		if let Some(node) = self.node.upgrade() {
			let _ = item_acceptor_client::release_item(&node, &item.uid);
		}

		self.accepted_registry.remove(item);
		self.accepted_aliases.remove(&item.uid);
	}
}
impl Aspect for ItemAcceptor {
	const NAME: &'static str = "ItemAcceptor";
}
impl ItemAcceptorAspect for ItemAcceptor {}
impl Drop for ItemAcceptor {
	fn drop(&mut self) {
		self.type_info.acceptors.remove(self);
		for item in self.accepted_registry.get_valid_contents() {
			release(&item);
		}
		if let Some(ui) = self.type_info.ui.lock().upgrade() {
			ui.handle_destroy_acceptor(self);
		}
	}
}

pub fn register_item_ui_flex(
	calling_client: Arc<Client>,
	type_info: &'static TypeInfo,
) -> Result<()> {
	let ui = Node::create_parent_name(&calling_client, "/item", type_info.type_name, true)
		.add_to_scenegraph()?;
	ItemUI::add_to(&ui, type_info)?;
	Ok(())
}
fn create_item_acceptor_flex(
	calling_client: Arc<Client>,
	name: String,
	parent: Arc<Node>,
	transform: Transform,
	type_info: &'static TypeInfo,
	field: Arc<Node>,
) -> Result<()> {
	let space = parent.get_aspect::<Spatial>()?;
	let field = field.get_aspect::<Field>()?;
	let transform = transform.to_mat4(true, true, false);

	let node = Node::create_parent_name(
		&calling_client,
		&format!("/item/{}/acceptor", type_info.type_name),
		&name,
		true,
	)
	.add_to_scenegraph()?;
	Spatial::add_to(&node, Some(space.clone()), transform, false);
	ItemAcceptor::add_to(&node, type_info, field);
	Ok(())
}

struct ItemInterface;
// create_interface!(ItemInterface, ItemInterfaceAspect, "/item");

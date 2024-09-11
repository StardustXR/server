pub mod camera;
pub mod panel;

use self::camera::CameraItem;
use self::panel::PanelItemTrait;
use super::alias::AliasList;
use super::fields::{Field, FIELD_ALIAS_INFO};
use super::spatial::Spatial;
use super::{Alias, Aspect, Node};
use crate::core::client::Client;
use crate::core::registry::Registry;
use crate::nodes::alias::AliasInfo;
use crate::nodes::spatial::Transform;
use crate::nodes::spatial::SPATIAL_ASPECT_ALIAS_INFO;
use color_eyre::eyre::{ensure, Result};
use parking_lot::Mutex;
use std::hash::Hash;
use std::sync::{Arc, Weak};

stardust_xr_server_codegen::codegen_item_protocol!();

fn capture(item: &Arc<Item>, acceptor: &Arc<ItemAcceptor>) {
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
			ui.handle_release_item(item, acceptor);
		}
	}
}

pub struct TypeInfo {
	pub type_name: &'static str,
	pub alias_info: AliasInfo,
	pub ui_node_id: u64,
	pub ui: Mutex<Weak<ItemUI>>,
	pub items: Registry<Item>,
	pub acceptors: Registry<ItemAcceptor>,
	pub add_ui_aspect: fn(node: &Node),
	pub add_acceptor_aspect: fn(node: &Node),
	pub new_acceptor_fn: fn(node: &Node, acceptor: &Arc<Node>, acceptor_field: &Arc<Node>),
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
	spatial: Arc<Spatial>,
	type_info: &'static TypeInfo,
	captured_acceptor: Mutex<Weak<ItemAcceptor>>,
	pub specialization: ItemType,
}
impl Item {
	pub fn add_to(
		node: &Arc<Node>,
		type_info: &'static TypeInfo,
		specialization: ItemType,
	) -> Arc<Self> {
		let item = Item {
			spatial: node.aspects.get::<Spatial>().unwrap(),
			type_info,
			captured_acceptor: Default::default(),
			specialization,
		};
		let item = type_info.items.add(item);

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
	fn make_alias(&self, client: &Arc<Client>, alias_list: &AliasList) -> Result<Arc<Node>> {
		Alias::create(
			&self.spatial.node().unwrap(),
			client,
			self.type_info.alias_info.clone() + ITEM_ASPECT_ALIAS_INFO.clone(),
			Some(alias_list),
		)
	}
}
impl Aspect for Item {
	impl_aspect_for_item_aspect! {}
}
impl ItemAspect for Item {
	fn release(node: Arc<Node>, _calling_client: Arc<Client>) -> Result<()> {
		let item = node.get_aspect::<Item>()?;
		release(&item);
		Ok(())
	}
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
	Camera(Arc<CameraItem>),
	Panel(Arc<dyn PanelItemTrait>),
}
impl ItemType {
	fn send_ui_item_created(&self, node: &Node, item: &Arc<Node>) {
		match self {
			ItemType::Camera(c) => c.send_ui_item_created(node, item),
			ItemType::Panel(p) => p.send_ui_item_created(node, item),
		}
	}
	fn send_acceptor_item_created(&self, node: &Node, item: &Arc<Node>) {
		match self {
			ItemType::Camera(c) => c.send_acceptor_item_created(node, item),
			ItemType::Panel(p) => p.send_acceptor_item_created(node, item),
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
	item_aliases: AliasList,
	acceptor_aliases: AliasList,
	acceptor_field_aliases: AliasList,
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
			item_aliases: AliasList::default(),
			acceptor_aliases: AliasList::default(),
			acceptor_field_aliases: AliasList::default(),
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

		let Ok(item_alias) = item.make_alias(&client, &self.item_aliases) else {
			return;
		};

		item.specialization.send_ui_item_created(&node, &item_alias);
	}
	fn handle_capture_item(&self, item: &Item, acceptor: &ItemAcceptor) {
		let Some(item_alias) = self.item_aliases.get_from_aspect(item) else {
			return;
		};
		let Some(acceptor_alias) = self.acceptor_aliases.get_from_aspect(acceptor) else {
			return;
		};
		let _ = item_ui_client::capture_item(
			&self.node.upgrade().unwrap(),
			item_alias.id,
			acceptor_alias.id,
		);
	}
	fn handle_release_item(&self, item: &Item, acceptor: &ItemAcceptor) {
		let Some(item_alias) = self.item_aliases.get_from_aspect(item) else {
			return;
		};
		let Some(acceptor_alias) = self.acceptor_aliases.get_from_aspect(acceptor) else {
			return;
		};
		let _ = item_ui_client::release_item(
			&self.node.upgrade().unwrap(),
			item_alias.id,
			acceptor_alias.id,
		);
	}
	fn handle_destroy_item(&self, item: &Item) {
		let Some(item_alias) = self
			.item_aliases
			.get_from_original_node(item.spatial.node.clone())
		else {
			return;
		};
		let _ = item_ui_client::destroy_item(&self.node.upgrade().unwrap(), item_alias.id);
		self.item_aliases.remove_aspect(item);
	}
	fn handle_create_acceptor(&self, acceptor: &ItemAcceptor) {
		let Some(node) = self.node.upgrade() else {
			return;
		};
		let Some(client) = node.get_client() else {
			return;
		};

		let Some(acceptor_node) = acceptor.spatial.node() else {
			return;
		};
		let Ok(acceptor_alias) = Alias::create(
			&acceptor_node,
			&client,
			ITEM_ACCEPTOR_ASPECT_ALIAS_INFO.clone(),
			Some(&self.acceptor_aliases),
		) else {
			return;
		};

		let Some(acceptor_field_node) = acceptor.field.spatial.node() else {
			return;
		};
		let Ok(acceptor_field_alias) = Alias::create(
			&acceptor_field_node,
			&client,
			FIELD_ALIAS_INFO.clone(),
			Some(&self.acceptor_aliases),
		) else {
			return;
		};

		(acceptor.type_info.new_acceptor_fn)(&node, &acceptor_alias, &acceptor_field_alias);
	}
	fn handle_destroy_acceptor(&self, acceptor: &ItemAcceptor) {
		let acceptor_alias = self.acceptor_aliases.get_from_aspect(acceptor).unwrap();
		let _ = item_ui_client::destroy_acceptor(&self.node.upgrade().unwrap(), acceptor_alias.id);

		self.acceptor_aliases
			.remove_aspect(acceptor.spatial.as_ref());
		self.acceptor_field_aliases
			.remove_aspect(acceptor.field.as_ref());
	}
}
impl Aspect for ItemUI {
	impl_aspect_for_item_ui_aspect! {}
}
impl Drop for ItemUI {
	fn drop(&mut self) {
		*self.type_info.ui.lock() = Weak::new();
	}
}

pub struct ItemAcceptor {
	spatial: Arc<Spatial>,
	pub type_info: &'static TypeInfo,
	field: Arc<Field>,
	accepted_aliases: AliasList,
	accepted_registry: Registry<Item>,
}
impl ItemAcceptor {
	fn add_to(node: &Arc<Node>, type_info: &'static TypeInfo, field: Arc<Field>) {
		let acceptor = type_info.acceptors.add(ItemAcceptor {
			spatial: node.get_aspect::<Spatial>().unwrap(),
			type_info,
			field,
			accepted_aliases: AliasList::default(),
			accepted_registry: Registry::new(),
		});
		if let Some(ui) = type_info.ui.lock().upgrade() {
			ui.handle_create_acceptor(&acceptor);
		}
		node.add_aspect_raw(acceptor.clone());
	}

	fn handle_capture(&self, item: &Arc<Item>) {
		let Some(node) = self.spatial.node() else {
			return;
		};
		let Some(client) = node.get_client() else {
			return;
		};

		self.accepted_registry.add_raw(item);
		let Ok(alias_node) = item.make_alias(&client, &self.accepted_aliases) else {
			return;
		};

		item.specialization
			.send_acceptor_item_created(&node, &alias_node);
	}
	fn handle_release(&self, item: &Item) {
		self.accepted_registry.remove(item);
		self.accepted_aliases.remove_aspect(item);

		let Some(node) = self.spatial.node() else {
			return;
		};
		let alias = self.accepted_aliases.get_from_aspect(item).unwrap();
		let _ = item_acceptor_client::release_item(&node, alias.id);
	}
}
impl Aspect for ItemAcceptor {
	impl_aspect_for_item_acceptor_aspect! {}
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
	let ui = Node::from_id(&calling_client, type_info.ui_node_id, true).add_to_scenegraph()?;
	ItemUI::add_to(&ui, type_info)?;
	(type_info.add_ui_aspect)(&ui);
	Ok(())
}
fn create_item_acceptor_flex(
	calling_client: Arc<Client>,
	id: u64,
	parent: Arc<Node>,
	transform: Transform,
	type_info: &'static TypeInfo,
	field: Arc<Node>,
) -> Result<()> {
	let space = parent.get_aspect::<Spatial>()?;
	let field = field.get_aspect::<Field>()?;
	let transform = transform.to_mat4(true, true, false);

	let node = Node::from_id(&calling_client, id, true).add_to_scenegraph()?;
	Spatial::add_to(&node, Some(space.clone()), transform, false);
	ItemAcceptor::add_to(&node, type_info, field);
	(type_info.add_acceptor_aspect)(&node);
	Ok(())
}

fn acceptor_capture_item_flex(node: Arc<Node>, item: Arc<Node>) -> Result<()> {
	let acceptor = node.get_aspect::<ItemAcceptor>()?;
	let item = item.get_aspect::<Item>()?;
	capture(&item, &acceptor);

	Ok(())
}

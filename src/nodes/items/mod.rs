mod environment;

use self::environment::{EnvironmentItem, ITEM_TYPE_INFO_ENVIRONMENT};
use super::fields::Field;
use super::spatial::{find_spatial_parent, parse_transform, Spatial};
use super::{Alias, Node};
use crate::core::client::{Client, INTERNAL_CLIENT};
use crate::core::node_collections::LifeLinkedNodeList;
use crate::core::registry::Registry;
use crate::nodes::alias::AliasInfo;
use crate::nodes::fields::find_field;
#[cfg(feature = "wayland")]
use crate::wayland::panel_item::{PanelItem, ITEM_TYPE_INFO_PANEL};
use anyhow::{anyhow, ensure, Result};
use lazy_static::lazy_static;
use nanoid::nanoid;
use parking_lot::Mutex;
use serde::Deserialize;
use stardust_xr::schemas::flex::{deserialize, serialize};
use stardust_xr::values::Transform;

use std::ops::Deref;
use std::sync::{Arc, Weak};

lazy_static! {
	static ref ITEM_ALIAS_LOCAL_SIGNALS: Vec<&'static str> = vec![
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

fn capture(item: &Arc<Item>, acceptor: &Arc<ItemAcceptor>) {
	if item.captured_acceptor.lock().upgrade().is_some() {
		release(item);
	}
	*item.captured_acceptor.lock() = Arc::downgrade(acceptor);
	acceptor.handle_capture(item);
	if let Some(ui) = item.type_info.ui.lock().upgrade() {
		ui.handle_capture_item(item, acceptor);
	}
}
fn release(item: &Arc<Item>) {
	if let Some(acceptor) = item.captured_acceptor.lock().upgrade() {
		*item.captured_acceptor.lock() = Weak::default();
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
}

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
		type_info: &'static TypeInfo,
		specialization: ItemType,
	) -> Arc<Self> {
		let item = Item {
			node: Arc::downgrade(node),
			uid: nanoid!(),
			type_info,
			captured_acceptor: Default::default(),
			specialization,
		};
		let item = type_info.items.add(item);

		node.add_local_signal("release", Item::release_flex);
		if let Some(ui) = type_info.ui.lock().upgrade() {
			ui.handle_create_item(&item);
		}
		let _ = node.item.set(item.clone());
		item
	}
	fn make_alias(&self, client: &Arc<Client>, parent: &str) -> (Arc<Node>, Arc<Alias>) {
		let node = Alias::create(
			client,
			parent,
			&self.uid,
			&self.node.upgrade().unwrap(),
			AliasInfo {
				local_signals: [
					&self.type_info.aliased_local_signals,
					ITEM_ALIAS_LOCAL_SIGNALS.as_slice(),
				]
				.concat(),
				local_methods: [
					&self.type_info.aliased_local_methods,
					ITEM_ALIAS_LOCAL_METHODS.as_slice(),
				]
				.concat(),
				remote_signals: [
					&self.type_info.aliased_remote_signals,
					ITEM_ALIAS_REMOTE_SIGNALS.as_slice(),
				]
				.concat(),
			},
		);
		let alias = node.alias.get().unwrap().clone();
		(node, alias)
	}

	fn release_flex(node: &Node, _calling_client: Arc<Client>, _data: &[u8]) -> Result<()> {
		let item = node.get_aspect("Item", "item", |n| &n.item)?;
		release(item);

		Ok(())
	}
}
impl Drop for Item {
	fn drop(&mut self) {
		self.type_info.items.remove(self);
		if let Some(ui) = self.type_info.ui.lock().upgrade() {
			ui.handle_destroy_item(self);
		}
	}
}

pub trait ItemSpecialization {
	fn serialize_start_data(&self, id: &str) -> Vec<u8>;
}

pub enum ItemType {
	Environment(EnvironmentItem),
	#[cfg(feature = "wayland")]
	Panel(PanelItem),
}
impl Deref for ItemType {
	type Target = dyn ItemSpecialization;

	fn deref(&self) -> &Self::Target {
		match self {
			ItemType::Environment(item) => item,
			#[cfg(feature = "wayland")]
			ItemType::Panel(item) => item,
		}
	}
}

pub struct ItemUI {
	node: Weak<Node>,
	type_info: &'static TypeInfo,
	aliases: LifeLinkedNodeList,
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
			aliases: Default::default(),
		});
		*type_info.ui.lock() = Arc::downgrade(&ui);
		let _ = node.item_ui.set(ui.clone());

		for item in type_info.items.get_valid_contents() {
			ui.handle_create_item(&item);
		}
		Ok(())
	}
	fn send_state(&self, state: &str, name: &str) {
		let _ = self
			.node
			.upgrade()
			.unwrap()
			.send_remote_signal(state, flexbuffers::singleton(name).as_slice());
	}

	fn handle_create_item(&self, item: &Item) {
		let Some(node) = self.node.upgrade() else { return };
		let Some(client) = node.get_client() else { return };

		let (alias_node, _) = item.make_alias(&client, &(node.get_path().to_string() + "/item"));
		self.aliases.add(Arc::downgrade(&alias_node));

		let _ = node.send_remote_signal(
			"create_item",
			&item.specialization.serialize_start_data(&item.uid),
		);
	}
	fn handle_destroy_item(&self, item: &Item) {
		self.send_state("destroy_item", item.uid.as_str());
	}
	fn handle_capture_item(&self, item: &Item, acceptor: &ItemAcceptor) {
		let Some(node) = self.node.upgrade() else { return };

		let _ = node.send_remote_signal(
			"capture_item",
			&serialize((item.uid.as_str(), acceptor.uid.as_str())).unwrap(),
		);
	}
	fn handle_release_item(&self, item: &Item, acceptor: &ItemAcceptor) {
		let Some(node) = self.node.upgrade() else { return };

		let _ = node.send_remote_signal(
			"release_item",
			&serialize((item.uid.as_str(), acceptor.uid.as_str())).unwrap(),
		);
	}
	fn handle_create_acceptor(&self, acceptor: &ItemAcceptor) {
		let Some(node) = self.node.upgrade() else { return };
		let Some(client) = node.get_client() else { return };

		let (alias, field_alias) = acceptor.make_aliases(
			&client,
			&format!("/item/{}/acceptor", self.type_info.type_name),
		);
		self.aliases.add(Arc::downgrade(&alias));
		self.aliases.add(Arc::downgrade(&field_alias));
		let _ = node.send_remote_signal("create_acceptor", &serialize(&acceptor.uid).unwrap());
	}
	fn handle_destroy_acceptor(&self, acceptor: &ItemAcceptor) {
		self.send_state("destroy_acceptor", acceptor.uid.as_str());
	}
}
impl Drop for ItemUI {
	fn drop(&mut self) {
		*self.type_info.ui.lock() = Weak::new();
	}
}

pub struct ItemAcceptor {
	uid: String,
	node: Weak<Node>,
	type_info: &'static TypeInfo,
	field: Arc<Field>,
	accepted: Registry<Item>,
}
impl ItemAcceptor {
	fn add_to(node: &Arc<Node>, type_info: &'static TypeInfo, field: Arc<Field>) {
		let acceptor = type_info.acceptors.add(ItemAcceptor {
			uid: nanoid!(),
			node: Arc::downgrade(node),
			type_info,
			field,
			accepted: Registry::new(),
		});
		node.add_local_signal("capture", ItemAcceptor::capture_flex);
		if let Some(ui) = type_info.ui.lock().upgrade() {
			ui.handle_create_acceptor(&acceptor);
		}
		let _ = node.item_acceptor.set(acceptor);
	}

	fn capture_flex(node: &Node, calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		let acceptor = node.get_aspect("Item acceptor", "item acceptor", |n| &n.item_acceptor)?;
		let item_path: &str = deserialize(data)?;
		let item_node = calling_client.get_node("Item", item_path)?;
		let item = item_node.get_aspect("Item", "item", |n| &n.item)?;
		capture(item, acceptor);

		Ok(())
	}
	fn make_aliases(&self, client: &Arc<Client>, parent: &str) -> (Arc<Node>, Arc<Node>) {
		let acceptor_node = &self.node.upgrade().unwrap();
		let acceptor_alias = Alias::create(
			client,
			parent,
			&self.uid,
			acceptor_node,
			AliasInfo {
				local_signals: vec!["capture"],
				..Default::default()
			},
		);
		let acceptor_field_alias = Alias::create(
			client,
			acceptor_alias.get_path(),
			"field",
			&self.field.spatial_ref().node.upgrade().unwrap(),
			AliasInfo::default(),
		);

		(acceptor_alias, acceptor_field_alias)
	}
	fn send_event(&self, state: &str, name: &str) {
		let _ = self
			.node
			.upgrade()
			.unwrap()
			.send_remote_signal(state, flexbuffers::singleton(name).as_slice());
	}
	fn handle_capture(&self, item: &Arc<Item>) {
		self.accepted.add_raw(item);
		self.send_event("capture", item.uid.as_str());
	}
	fn handle_release(&self, item: &Item) {
		self.accepted.remove(item);
		self.send_event("release", item.uid.as_str());
	}
}
impl Drop for ItemAcceptor {
	fn drop(&mut self) {
		self.type_info.acceptors.remove(self);
		if let Some(ui) = self.type_info.ui.lock().upgrade() {
			ui.handle_destroy_acceptor(self);
		}
	}
}

pub fn create_interface(client: &Arc<Client>) {
	let node = Node::create(client, "", "item", false);
	node.add_local_signal(
		"create_environment_item",
		environment::create_environment_item_flex,
	);
	node.add_local_signal("register_item_ui", register_item_ui_flex);
	node.add_local_signal("create_item_acceptor", create_item_acceptor_flex);
	node.add_to_scenegraph();
}

fn type_info(name: &str) -> Result<&'static TypeInfo> {
	match name {
		"environment" => Ok(&ITEM_TYPE_INFO_ENVIRONMENT),
		#[cfg(feature = "wayland")]
		"panel" => Ok(&ITEM_TYPE_INFO_PANEL),
		_ => Err(anyhow!("Invalid item type")),
	}
}

pub fn register_item_ui_flex(_node: &Node, calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
	#[derive(Deserialize)]
	struct RegisterItemUIInfo<'a> {
		item_type: &'a str,
	}
	let info: RegisterItemUIInfo = deserialize(data)?;
	let type_info = type_info(info.item_type)?;
	let ui = Node::create(&calling_client, "/item", type_info.type_name, true).add_to_scenegraph();
	ItemUI::add_to(&ui, type_info)?;
	Ok(())
}

fn create_item_acceptor_flex(_node: &Node, calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
	#[derive(Deserialize)]
	struct CreateItemAcceptorInfo<'a> {
		name: &'a str,
		parent_path: &'a str,
		transform: Transform,
		field_path: &'a str,
		item_type: &'a str,
	}
	let info: CreateItemAcceptorInfo = deserialize(data)?;
	let space = find_spatial_parent(&calling_client, info.parent_path)?;
	let transform = parse_transform(info.transform, true, true, false)?;
	let field = find_field(&calling_client, info.field_path)?;
	let type_info = type_info(info.item_type)?;

	let node = Node::create(
		&INTERNAL_CLIENT,
		&format!("/item/{}/acceptor", type_info.type_name),
		info.name,
		true,
	)
	.add_to_scenegraph();
	Spatial::add_to(&node, Some(space), transform, false)?;
	ItemAcceptor::add_to(&node, type_info, field);
	Ok(())
}

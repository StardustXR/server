mod environment;

use self::environment::EnvironmentItem;

use super::fields::Field;
use super::spatial::{get_spatial_parent_flex, parse_transform, Spatial};
use super::{Alias, Node};
use crate::core::client::{Client, INTERNAL_CLIENT};
use crate::core::nodelist::LifeLinkedNodeList;
use crate::core::registry::Registry;
use crate::wayland::panel_item::{register_panel_item_ui_flex, PanelItem};
use anyhow::{anyhow, ensure, Result};
use lazy_static::lazy_static;
use nanoid::nanoid;
use parking_lot::Mutex;
use stardust_xr::flex::flexbuffer_from_vector_arguments;
use std::ops::Deref;
use std::sync::{Arc, Weak};

lazy_static! {
	static ref ITEM_ALIAS_LOCAL_SIGNALS: Vec<&'static str> = vec![
		"getTransform",
		"setTransform",
		"setSpatialParent",
		"setSpatialParentInPlace",
		"setZoneable",
		"release",
	];
	static ref ITEM_ALIAS_LOCAL_METHODS: Vec<&'static str> = vec!["captureInto"];
	static ref ITEM_ALIAS_REMOTE_SIGNALS: Vec<&'static str> = vec![];
	static ref ITEM_ALIAS_REMOTE_METHODS: Vec<&'static str> = vec![];
	static ref ITEM_TYPE_INFO_ENVIRONMENT: TypeInfo = TypeInfo {
		type_name: "environment",
		aliased_local_signals: vec!["applySkyTex", "applySkyLight"],
		aliased_local_methods: vec![],
		aliased_remote_signals: vec![],
		aliased_remote_methods: vec![],
		ui: Default::default(),
		items: Registry::new(),
		acceptors: Registry::new(),
	};
}

fn capture(item: &Arc<Item>, acceptor: &Arc<ItemAcceptor>) {
	if item.captured_acceptor.lock().upgrade().is_some() {
		release(item);
	}
	*item.captured_acceptor.lock() = Arc::downgrade(acceptor);
	acceptor.handle_capture(item);
	if let Some(ui) = item.type_info.ui.lock().upgrade() {
		ui.handle_capture(item);
	}
}
fn release(item: &Arc<Item>) {
	if let Some(acceptor) = item.captured_acceptor.lock().upgrade() {
		*item.captured_acceptor.lock() = Weak::default();
		acceptor.handle_release(item);
		if let Some(ui) = item.type_info.ui.lock().upgrade() {
			ui.handle_release(item);
		}
	}
}

pub struct TypeInfo {
	pub type_name: &'static str,
	pub aliased_local_signals: Vec<&'static str>,
	pub aliased_local_methods: Vec<&'static str>,
	pub aliased_remote_signals: Vec<&'static str>,
	pub aliased_remote_methods: Vec<&'static str>,
	pub ui: Mutex<Weak<ItemUI>>,
	pub items: Registry<Item>,
	pub acceptors: Registry<ItemAcceptor>,
}

fn capture_into_flex(node: &Node, calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
	let acceptor_path = flexbuffers::Reader::get_root(data)?
		.get_str()
		.map_err(|_| anyhow!("Acceptor path is not a string"))?;
	let acceptor = calling_client
		.scenegraph
		.get_node(acceptor_path)
		.ok_or_else(|| anyhow!("Acceptor node not found"))?;
	let acceptor = acceptor
		.item_acceptor
		.get()
		.ok_or_else(|| anyhow!("Acceptor node is not an acceptor!"))?;
	capture(node.item.get().unwrap(), acceptor);
	Ok(())
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
		node.add_local_signal("captureInto", capture_into_flex);
		let item = Item {
			node: Arc::downgrade(node),
			uid: nanoid!(),
			type_info,
			captured_acceptor: Default::default(),
			specialization,
		};
		let item = type_info.items.add(item);
		if let Some(ui) = type_info.ui.lock().upgrade() {
			ui.handle_create_item(&item);
		}
		let _ = node.item.set(item.clone());
		item
	}
	fn make_alias(&self, client: &Arc<Client>, parent: &str) -> (Arc<Node>, Arc<Alias>) {
		let node = Node::create(client, parent, &self.uid, true).add_to_scenegraph();
		let alias = Alias::add_to(
			&node,
			&self.node.upgrade().unwrap(),
			[
				&self.type_info.aliased_local_signals,
				ITEM_ALIAS_LOCAL_SIGNALS.as_slice(),
			]
			.concat(),
			[
				&self.type_info.aliased_local_methods,
				ITEM_ALIAS_LOCAL_METHODS.as_slice(),
			]
			.concat(),
			[
				&self.type_info.aliased_remote_signals,
				ITEM_ALIAS_REMOTE_SIGNALS.as_slice(),
			]
			.concat(),
			[
				&self.type_info.aliased_remote_methods,
				ITEM_ALIAS_REMOTE_METHODS.as_slice(),
			]
			.concat(),
		);
		(node, alias)
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
	fn serialize_start_data(&self, vec: &mut flexbuffers::VectorBuilder);
}

pub enum ItemType {
	Environment(EnvironmentItem),
	Panel(PanelItem),
}
impl Deref for ItemType {
	type Target = dyn ItemSpecialization;

	fn deref(&self) -> &Self::Target {
		match self {
			ItemType::Environment(item) => item,
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
		let node = self.node.upgrade().unwrap();
		let (alias_node, _) =
			item.make_alias(&node.get_client(), &(node.get_path().to_string() + "/item"));
		self.aliases.add(Arc::downgrade(&alias_node));

		let _ = node.send_remote_signal(
			"create",
			&flexbuffer_from_vector_arguments(|vec| {
				vec.push(item.uid.as_str());
				let mut start_data_vec = vec.start_vector();
				item.specialization
					.serialize_start_data(&mut start_data_vec);
			}),
		);
	}
	fn handle_destroy_item(&self, item: &Item) {
		self.send_state("destroy", item.uid.as_str());
	}
	fn handle_capture(&self, item: &Item) {
		self.send_state("capture", item.uid.as_str());
	}
	fn handle_release(&self, item: &Item) {
		self.send_state("release", item.uid.as_str());
	}
	fn handle_create_acceptor(&self, acceptor: &ItemAcceptor) {
		let node = self.node.upgrade().unwrap();
		let aliases = acceptor.make_aliases(
			&node.get_client(),
			&format!("/item/{}/acceptor", self.type_info.type_name),
		);
		aliases
			.iter()
			.for_each(|alias| self.aliases.add(Arc::downgrade(alias)));
	}
	fn handle_destroy_acceptor(&self, _acceptor: &ItemAcceptor) {}
}
impl Drop for ItemUI {
	fn drop(&mut self) {
		*self.type_info.ui.lock() = Weak::new();
	}
}

pub struct ItemAcceptor {
	node: Weak<Node>,
	type_info: &'static TypeInfo,
	field: Mutex<Weak<Field>>,
	accepted: Registry<Item>,
}
impl ItemAcceptor {
	fn add_to(node: &Arc<Node>, type_info: &'static TypeInfo, field: Weak<Field>) {
		let acceptor = type_info.acceptors.add(ItemAcceptor {
			node: Arc::downgrade(node),
			type_info,
			field: Mutex::new(field),
			accepted: Registry::new(),
		});
		if let Some(ui) = type_info.ui.lock().upgrade() {
			ui.handle_create_acceptor(&acceptor);
		}
		let _ = node.item_acceptor.set(acceptor);
	}
	fn make_aliases(&self, client: &Arc<Client>, parent: &str) -> Vec<Arc<Node>> {
		let mut aliases = Vec::new();
		let acceptor_node = &self.node.upgrade().unwrap();
		let acceptor_alias =
			Node::create(client, parent, acceptor_node.uid.as_str(), true).add_to_scenegraph();
		Alias::add_to(
			&acceptor_alias,
			acceptor_node,
			vec!["release"],
			vec![],
			vec![],
			vec![],
		);
		if let Some(field) = self.field.lock().upgrade() {
			let acceptor_field_alias =
				Node::create(client, acceptor_alias.get_path(), "field", true).add_to_scenegraph();
			Alias::add_to(
				&acceptor_field_alias,
				&field.spatial_ref().node.upgrade().unwrap(),
				vec![],
				vec![],
				vec![],
				vec![],
			);
			aliases.push(acceptor_field_alias);
		}
		aliases.push(acceptor_alias);
		aliases
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
	node.add_local_signal("createEnvironmentItem", create_environment_item_flex);
	node.add_local_signal(
		"registerEnvironmentItemUI",
		register_environment_item_ui_flex,
	);
	node.add_local_signal("registerPanelItemUI", register_panel_item_ui_flex);
	node.add_local_signal(
		"createEnvironmentItemAcceptor",
		create_environment_item_acceptor_flex,
	);
	node.add_to_scenegraph();
}

pub fn create_environment_item_flex(
	_node: &Node,
	calling_client: Arc<Client>,
	data: &[u8],
) -> Result<()> {
	let root = flexbuffers::Reader::get_root(data)?;
	let flex_vec = root.get_vector()?;
	let parent_name = format!("/item/{}/item/", ITEM_TYPE_INFO_ENVIRONMENT.type_name);
	let node = Node::create(
		&INTERNAL_CLIENT,
		&parent_name,
		flex_vec.idx(0).get_str()?,
		true,
	);
	let space = get_spatial_parent_flex(&calling_client, flex_vec.idx(1).get_str()?)?;
	let transform = parse_transform(flex_vec.idx(2), true, true, false)?;
	let node = node.add_to_scenegraph();
	Spatial::add_to(&node, None, transform * space.global_transform())?;
	EnvironmentItem::add_to(&node, flex_vec.idx(3).get_str()?.to_string());
	node.item
		.get()
		.unwrap()
		.make_alias(&calling_client, &parent_name);
	Ok(())
}

pub fn create_item_acceptor_flex(
	calling_client: Arc<Client>,
	data: &[u8],
	type_info: &'static TypeInfo,
) -> Result<()> {
	let root = flexbuffers::Reader::get_root(data)?;
	let flex_vec = root.get_vector()?;
	let parent_name = format!("/item/{}/acceptor/", ITEM_TYPE_INFO_ENVIRONMENT.type_name);
	let space = get_spatial_parent_flex(&calling_client, flex_vec.idx(1).get_str()?)?;
	let transform = parse_transform(flex_vec.idx(2), true, true, false)?;
	let field = calling_client
		.scenegraph
		.get_node(flex_vec.idx(3).get_str()?)
		.ok_or_else(|| anyhow!("Field node not found"))?;
	let field = field
		.field
		.get()
		.ok_or_else(|| anyhow!("Field node is not a field"))?;

	let node = Node::create(
		&INTERNAL_CLIENT,
		&parent_name,
		flex_vec.idx(0).get_str()?,
		true,
	)
	.add_to_scenegraph();
	Spatial::add_to(&node, None, transform * space.global_transform())?;
	ItemAcceptor::add_to(&node, type_info, Arc::downgrade(field));
	node.item
		.get()
		.unwrap()
		.make_alias(&calling_client, &parent_name);
	Ok(())
}

pub fn create_environment_item_acceptor_flex(
	_node: &Node,
	calling_client: Arc<Client>,
	data: &[u8],
) -> Result<()> {
	create_item_acceptor_flex(calling_client, data, &ITEM_TYPE_INFO_ENVIRONMENT)
}

pub fn register_item_ui_flex(
	calling_client: Arc<Client>,
	type_info: &'static TypeInfo,
) -> Result<()> {
	let ui = Node::create(&calling_client, "/item", type_info.type_name, true).add_to_scenegraph();
	ItemUI::add_to(&ui, type_info)?;
	Ok(())
}

pub fn register_environment_item_ui_flex(
	_node: &Node,
	calling_client: Arc<Client>,
	_data: &[u8],
) -> Result<()> {
	register_item_ui_flex(calling_client, &ITEM_TYPE_INFO_ENVIRONMENT)
}

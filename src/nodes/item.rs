use super::core::{Alias, Node};
use super::field::Field;
use super::spatial::{get_spatial_parent_flex, get_transform_pose_flex, Spatial};
use crate::core::client::{Client, INTERNAL_CLIENT};
use crate::core::nodelist::LifeLinkedNodeList;
use crate::core::registry::Registry;
use anyhow::{anyhow, ensure, Result};
use lazy_static::lazy_static;
use nanoid::nanoid;
use parking_lot::Mutex;
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
	static ref ITEM_ALIAS_LOCAL_METHODS: Vec<&'static str> = vec![];
	static ref ITEM_ALIAS_REMOTE_SIGNALS: Vec<&'static str> = vec![];
	static ref ITEM_ALIAS_REMOTE_METHODS: Vec<&'static str> = vec![];
	static ref ITEM_TYPE_INFO_ENVIRONMENT: TypeInfo = TypeInfo {
		type_name: "environment",
		aliased_local_signals: vec!["applySkyTex", "applySkyLight"],
		aliased_local_methods: vec![],
		aliased_remote_signals: vec![],
		aliased_remote_methods: vec![],
		ui: Default::default(),
		items: Default::default(),
		acceptors: Default::default(),
	};
	static ref ITEM_TYPE_INFO_PANEL: TypeInfo = TypeInfo {
		type_name: "panel",
		aliased_local_signals: vec![
			"applySurfaceMaterial",
			"setPointerActive",
			"setPointerPosition",
			"setPointerButtonPressed",
			"scrollPointerAxis",
			"touchDown",
			"touchMove",
			"touchUp",
			"setKeyboardActive",
			"setKeymap",
			"setKeyState",
			"setKeyModStates",
			"setKeyRepeat",
			"resize",
			"close",
		],
		aliased_local_methods: vec![],
		aliased_remote_signals: vec![],
		aliased_remote_methods: vec![],
		ui: Default::default(),
		items: Default::default(),
		acceptors: Default::default(),
	};
}

pub struct TypeInfo {
	type_name: &'static str,
	aliased_local_signals: Vec<&'static str>,
	aliased_local_methods: Vec<&'static str>,
	aliased_remote_signals: Vec<&'static str>,
	aliased_remote_methods: Vec<&'static str>,
	ui: Mutex<Weak<ItemUI>>,
	items: Registry<ItemType>,
	acceptors: Registry<ItemAcceptor>,
}

pub trait Item {
	fn get_type_info(&self) -> &'static TypeInfo;
	fn get_data(&self) -> &ItemData;
	fn get_node(&self) -> Arc<Node> {
		self.get_data().node.upgrade().unwrap()
	}
	fn make_alias(&self, client: &Arc<Client>, parent: &str) -> Arc<Alias> {
		let type_info = self.get_type_info();
		let alias = Node::create(client, parent, "", true).add_to_scenegraph();
		Alias::add_to(
			&alias,
			&self.get_data().node.upgrade().unwrap(),
			[
				&type_info.aliased_local_signals,
				ITEM_ALIAS_LOCAL_SIGNALS.as_slice(),
			]
			.concat(),
			[
				&type_info.aliased_local_methods,
				ITEM_ALIAS_LOCAL_METHODS.as_slice(),
			]
			.concat(),
			[
				&type_info.aliased_remote_signals,
				ITEM_ALIAS_REMOTE_SIGNALS.as_slice(),
			]
			.concat(),
			[
				&type_info.aliased_remote_methods,
				ITEM_ALIAS_REMOTE_METHODS.as_slice(),
			]
			.concat(),
		)
	}

	fn capture(&self, acceptor: &Arc<ItemAcceptor>)
	where
		Self: std::marker::Sized,
	{
		let node = self.get_node();
		if self.get_data().captured_acceptor.lock().upgrade().is_some() {
			self.release();
		}
		*self.get_data().captured_acceptor.lock() = Arc::downgrade(acceptor);
		acceptor.accepted.add_raw(node.item.get().unwrap());
		if let Some(ui) = self.get_type_info().ui.lock().upgrade() {
			ui.handle_capture(self);
		}
	}
	fn release(&self)
	where
		Self: std::marker::Sized,
	{
		if let Some(acceptor) = self.get_data().captured_acceptor.lock().upgrade() {
			let node = self.get_node();
			*self.get_data().captured_acceptor.lock() = Weak::default();
			acceptor.accepted.remove(node.item.get().unwrap());
			if let Some(ui) = self.get_type_info().ui.lock().upgrade() {
				ui.handle_release(self);
			}
		}
	}
}

pub struct ItemData {
	node: Weak<Node>,
	uid: String,
	captured_acceptor: Mutex<Weak<ItemAcceptor>>,
}
impl ItemData {
	fn new(node: &Arc<Node>) -> Self {
		ItemData {
			node: Arc::downgrade(node),
			uid: nanoid!(),
			captured_acceptor: Default::default(),
		}
	}
}

pub enum ItemType {
	Environment(EnvironmentItem),
}
impl Deref for ItemType {
	type Target = dyn Item;
	fn deref(&self) -> &Self::Target {
		match self {
			ItemType::Environment(item) => item,
		}
	}
}

pub struct EnvironmentItem {
	data: ItemData,
	path: String,
}
impl EnvironmentItem {
	pub fn add_to(node: &Arc<Node>, path: String) {
		let item = Arc::new(ItemType::Environment(EnvironmentItem {
			data: ItemData::new(node),
			path,
		}));
		let _ = node.item.set(item);
	}
}
impl Item for EnvironmentItem {
	fn get_type_info(&self) -> &'static TypeInfo {
		&ITEM_TYPE_INFO_ENVIRONMENT
	}
	fn get_data(&self) -> &ItemData {
		&self.data
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
		let _ = node.item_ui.set(ui);
		Ok(())
	}
	fn send_state(&self, state: &str, name: &str) {
		let _ = self
			.node
			.upgrade()
			.unwrap()
			.send_remote_signal(state, flexbuffers::singleton(name).as_slice());
	}

	fn handle_create(&self, item: &dyn Item) {
		let uid = item.get_data().uid.as_str();
		self.send_state("create", uid);
	}
	fn handle_capture(&self, item: &dyn Item) {
		self.send_state("capture", item.get_data().uid.as_str());
	}
	fn handle_release(&self, item: &dyn Item) {
		self.send_state("release", item.get_data().uid.as_str());
	}
	fn handle_destroy(&self, item: &dyn Item) {
		self.send_state("destroy", item.get_data().uid.as_str());
	}
}

pub struct ItemAcceptor {
	node: Weak<Node>,
	type_info: &'static TypeInfo,
	field: Mutex<Weak<Field>>,
	accepted: Registry<ItemType>,
}
impl ItemAcceptor {
	fn add_to(node: &Arc<Node>, type_info: &'static TypeInfo, field: Weak<Field>) {
		let acceptor = type_info.acceptors.add(ItemAcceptor {
			node: Arc::downgrade(node),
			type_info,
			field: Mutex::new(field),
			accepted: Default::default(),
		});
		let _ = node.item_acceptor.set(acceptor);
	}
	fn make_alias(&self, client: &Arc<Client>, parent: &str) {
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
		}
	}
}

pub fn create_interface(client: &Arc<Client>) {
	let node = Node::create(client, "", "item", false);
	node.add_local_signal("createEnvironmentItem", create_environment_item_flex);
	node.add_local_signal("createItemAcceptor", create_environment_item_acceptor_flex);
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
	let transform = get_transform_pose_flex(&flex_vec.idx(2), &flex_vec.idx(3))?;
	let node = node.add_to_scenegraph();
	Spatial::add_to(&node, None, transform * space.global_transform())?;
	EnvironmentItem::add_to(&node, flex_vec.idx(4).get_str()?.to_string());
	node.item
		.get()
		.unwrap()
		.make_alias(&calling_client, &parent_name);
	Ok(())
}

pub fn create_item_acceptor_flex(
	_node: &Node,
	calling_client: Arc<Client>,
	data: &[u8],
	type_info: &'static TypeInfo,
) -> Result<()> {
	let root = flexbuffers::Reader::get_root(data)?;
	let flex_vec = root.get_vector()?;
	let parent_name = format!("/item/{}/acceptor/", ITEM_TYPE_INFO_ENVIRONMENT.type_name);
	let space = get_spatial_parent_flex(&calling_client, flex_vec.idx(1).get_str()?)?;
	let transform = get_transform_pose_flex(&flex_vec.idx(2), &flex_vec.idx(3))?;
	let field = calling_client
		.scenegraph
		.get_node(flex_vec.idx(4).get_str()?)
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
	node: &Node,
	calling_client: Arc<Client>,
	data: &[u8],
) -> Result<()> {
	create_item_acceptor_flex(node, calling_client, data, &ITEM_TYPE_INFO_ENVIRONMENT)
}

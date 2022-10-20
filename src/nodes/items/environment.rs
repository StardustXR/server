use super::{Item, ItemSpecialization, ItemType, ITEM_TYPE_INFO_ENVIRONMENT};
use crate::{
	core::client::{Client, INTERNAL_CLIENT},
	nodes::{
		spatial::{find_spatial_parent, parse_transform, Spatial},
		Node,
	},
};
use anyhow::{anyhow, Result};
use serde::Deserialize;
use stardust_xr::{
	schemas::flex::{deserialize, serialize},
	values::Transform,
};
use std::sync::Arc;

pub struct EnvironmentItem {
	path: String,
}
impl EnvironmentItem {
	pub fn add_to(node: &Arc<Node>, path: String) {
		Item::add_to(
			node,
			&ITEM_TYPE_INFO_ENVIRONMENT,
			ItemType::Environment(EnvironmentItem { path }),
		);
		node.add_local_method("getPath", EnvironmentItem::get_path_flex);
	}

	fn get_path_flex(node: &Node, _calling_client: Arc<Client>, _data: &[u8]) -> Result<Vec<u8>> {
		let path: Result<String> = match &node.item.get().unwrap().specialization {
			ItemType::Environment(env) => Ok(env.path.clone()),
			_ => Err(anyhow!("")),
		};
		Ok(flexbuffers::singleton(path?.as_str()))
	}
}
impl ItemSpecialization for EnvironmentItem {
	fn serialize_start_data(&self, id: &str) -> Vec<u8> {
		serialize((id, self.path.as_str())).unwrap()
	}
}

pub(super) fn create_environment_item_flex(
	_node: &Node,
	calling_client: Arc<Client>,
	data: &[u8],
) -> Result<()> {
	#[derive(Deserialize)]
	struct CreateEnvironmentItemInfo<'a> {
		name: &'a str,
		parent_path: &'a str,
		transform: Transform,
		item_data: String,
	}
	let info: CreateEnvironmentItemInfo = deserialize(data)?;
	let parent_name = format!("/item/{}/item/", ITEM_TYPE_INFO_ENVIRONMENT.type_name);
	let node = Node::create(&INTERNAL_CLIENT, &parent_name, info.name, true);
	let space = find_spatial_parent(&calling_client, info.parent_path)?;
	let transform = parse_transform(info.transform, true, true, false)?;
	let node = node.add_to_scenegraph();
	Spatial::add_to(&node, None, transform * space.global_transform(), false)?;
	EnvironmentItem::add_to(&node, info.item_data);
	node.item
		.get()
		.unwrap()
		.make_alias(&calling_client, &parent_name);
	Ok(())
}

pub(super) fn create_environment_item_acceptor_flex(
	_node: &Node,
	calling_client: Arc<Client>,
	data: &[u8],
) -> Result<()> {
	super::create_item_acceptor_flex(calling_client, data, &ITEM_TYPE_INFO_ENVIRONMENT)
}

pub(super) fn register_environment_item_ui_flex(
	_node: &Node,
	calling_client: Arc<Client>,
	_data: &[u8],
) -> Result<()> {
	super::register_item_ui_flex(calling_client, &ITEM_TYPE_INFO_ENVIRONMENT)
}

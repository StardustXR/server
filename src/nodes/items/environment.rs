use super::{Item, ItemSpecialization, ItemType};
use crate::{
	core::{
		client::{Client, INTERNAL_CLIENT},
		registry::Registry,
	},
	nodes::{
		items::TypeInfo,
		spatial::{find_spatial_parent, parse_transform, Spatial},
		Node,
	},
};
use color_eyre::eyre::{eyre, Result};
use lazy_static::lazy_static;
use nanoid::nanoid;
use serde::Deserialize;
use stardust_xr::{
	schemas::flex::{deserialize, flexbuffers, serialize},
	values::Transform,
};
use std::sync::Arc;

lazy_static! {
	pub(super) static ref ITEM_TYPE_INFO_ENVIRONMENT: TypeInfo = TypeInfo {
		type_name: "environment",
		aliased_local_signals: vec!["apply_sky_tex", "apply_sky_light"],
		aliased_local_methods: vec![],
		aliased_remote_signals: vec![],
		ui: Default::default(),
		items: Registry::new(),
		acceptors: Registry::new(),
	};
}

pub struct EnvironmentItem {
	path: String,
}
impl EnvironmentItem {
	pub fn add_to(node: &Arc<Node>, path: String) {
		Item::add_to(
			node,
			nanoid!(),
			&ITEM_TYPE_INFO_ENVIRONMENT,
			ItemType::Environment(EnvironmentItem { path }),
		);
		node.add_local_method("get_path", EnvironmentItem::get_path_flex);
	}

	fn get_path_flex(node: &Node, _calling_client: Arc<Client>, _data: &[u8]) -> Result<Vec<u8>> {
		let ItemType::Environment(environment_item) = &node.item.get().unwrap().specialization else {
			return Err(eyre!("Wrong item type?"))
		};
		Ok(flexbuffers::singleton(environment_item.path.as_str()))
	}
}
impl ItemSpecialization for EnvironmentItem {
	fn serialize_start_data(&self, id: &str) -> Option<Vec<u8>> {
		serialize((id, self.path.as_str())).ok()
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
	let parent_name = format!("/item/{}/item", ITEM_TYPE_INFO_ENVIRONMENT.type_name);
	let space = find_spatial_parent(&calling_client, info.parent_path)?;
	let transform = parse_transform(info.transform, true, true, false);

	let node =
		Node::create(&INTERNAL_CLIENT, &parent_name, info.name, false).add_to_scenegraph()?;
	Spatial::add_to(&node, None, transform * space.global_transform(), false)?;
	EnvironmentItem::add_to(&node, info.item_data);
	node.item
		.get()
		.unwrap()
		.make_alias_named(&calling_client, &parent_name, info.name)?;
	Ok(())
}

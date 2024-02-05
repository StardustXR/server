use super::{Item, ItemType};
use crate::{
	core::{
		client::{Client, INTERNAL_CLIENT},
		registry::Registry,
		scenegraph::MethodResponseSender,
	},
	nodes::{
		items::TypeInfo,
		spatial::{parse_transform, Spatial, Transform},
		Message, Node,
	},
};
use color_eyre::eyre::{eyre, Result};
use lazy_static::lazy_static;
use nanoid::nanoid;
use serde::Deserialize;
use stardust_xr::schemas::flex::{deserialize, serialize};
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

	fn get_path_flex(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		_message: Message,
		response: MethodResponseSender,
	) {
		response.wrap_sync(move || {
			let ItemType::Environment(environment_item) =
				&node.get_aspect::<Item>().unwrap().specialization
			else {
				return Err(eyre!("Wrong item type?"));
			};
			Ok(serialize(environment_item.path.as_str())?.into())
		});
	}

	pub fn serialize_start_data(&self, id: &str) -> Result<Message> {
		Ok(serialize((id, self.path.as_str()))?.into())
	}
}

pub(super) fn create_environment_item_flex(
	_node: Arc<Node>,
	calling_client: Arc<Client>,
	message: Message,
) -> Result<()> {
	#[derive(Deserialize)]
	struct CreateEnvironmentItemInfo<'a> {
		name: &'a str,
		parent_path: &'a str,
		transform: Transform,
		item_data: String,
	}
	let info: CreateEnvironmentItemInfo = deserialize(message.as_ref())?;
	let parent_name = format!("/item/{}/item", ITEM_TYPE_INFO_ENVIRONMENT.type_name);
	let space = calling_client
		.get_node("Spatial parent", info.parent_path)?
		.get_aspect::<Spatial>()?;
	let transform = parse_transform(info.transform, true, true, false);

	let node = Node::create_parent_name(&INTERNAL_CLIENT, &parent_name, info.name, false)
		.add_to_scenegraph()?;
	Spatial::add_to(&node, None, transform * space.global_transform(), false);
	EnvironmentItem::add_to(&node, info.item_data);
	node.get_aspect::<Item>().unwrap().make_alias_named(
		&calling_client,
		&parent_name,
		info.name,
	)?;
	Ok(())
}

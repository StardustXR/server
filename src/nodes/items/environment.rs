use super::{Item, ItemSpecialization, ItemType, ITEM_TYPE_INFO_ENVIRONMENT};
use crate::{core::client::Client, nodes::Node};
use anyhow::{anyhow, Result};
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
	fn serialize_start_data(&self, vec: &mut flexbuffers::VectorBuilder) {
		vec.push(self.path.as_str());
	}
}

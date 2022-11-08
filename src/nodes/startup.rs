use crate::core::client::Client;

use super::Node;
use anyhow::{anyhow, Result};
use glam::Mat4;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use std::sync::Arc;

lazy_static::lazy_static! {
	pub static ref DESKTOP_STARTUP_IDS: Mutex<FxHashMap<String, StartupSettings>> = Default::default();
}

#[derive(Debug, Default, Clone)]
pub struct StartupSettings {
	pub transform: Mat4,
}
impl StartupSettings {
	pub fn add_to(node: &Arc<Node>) {
		node.startup_settings
			.set(Mutex::new(StartupSettings::default()))
			.unwrap();
	}

	fn set_root_flex(node: &Node, calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		let startup_id = flexbuffers::Reader::get_root(data)?.get_str()?;
		let spatial_node = calling_client
			.scenegraph
			.get_node(startup_id)
			.ok_or_else(|| anyhow!("Root spatial node does not exist"))?;
		let spatial = spatial_node
			.spatial
			.get()
			.ok_or_else(|| anyhow!("Root spatial node is not a spatial"))?;
		node.startup_settings.get().unwrap().lock().transform = spatial.global_transform();

		Ok(())
	}

	fn generate_desktop_startup_id_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		_data: &[u8],
	) -> Result<Vec<u8>> {
		let id = nanoid::nanoid!();
		let data = flexbuffers::singleton(id.as_str());
		DESKTOP_STARTUP_IDS
			.lock()
			.insert(id, node.startup_settings.get().unwrap().lock().clone());
		Ok(data)
	}
}

pub fn create_interface(client: &Arc<Client>) {
	let node = Node::create(client, "", "startup", false);
	node.add_local_signal("create_startup_settings", create_startup_settings_flex);
	node.add_to_scenegraph();
}

pub fn create_startup_settings_flex(
	_node: &Node,
	calling_client: Arc<Client>,
	data: &[u8],
) -> Result<()> {
	let name = flexbuffers::Reader::get_root(data)?.get_str()?;
	let node = Node::create(&calling_client, "/startup/settings", name, true).add_to_scenegraph();
	StartupSettings::add_to(&node);

	node.add_local_signal("set_root", StartupSettings::set_root_flex);
	node.add_local_method(
		"generate_desktop_startup_id",
		StartupSettings::generate_desktop_startup_id_flex,
	);

	Ok(())
}

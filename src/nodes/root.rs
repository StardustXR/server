use super::spatial::Spatial;
use super::Node;
use crate::core::client::Client;
use crate::core::registry::Registry;
use color_eyre::eyre::Result;
use glam::Mat4;
use stardust_xr::schemas::flex::{deserialize, serialize};
use tracing::instrument;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

static ROOT_REGISTRY: Registry<Root> = Registry::new();

pub struct Root {
	node: Arc<Node>,
	logic_step: AtomicBool,
}
impl Root {
	pub fn create(client: &Arc<Client>) -> Result<Arc<Self>> {
		let node = Node::create(client, "", "", false);
		node.add_local_signal("subscribe_logic_step", Root::subscribe_logic_step);
		node.add_local_signal("set_base_prefixes", Root::set_base_prefixes);
		let node = node.add_to_scenegraph()?;
		let _ = Spatial::add_to(
			&node,
			None,
			client
				.startup_settings
				.as_ref()
				.map(|settings| settings.transform)
				.unwrap_or(Mat4::IDENTITY),
			false,
		);

		Ok(ROOT_REGISTRY.add(Root {
			node,
			logic_step: AtomicBool::from(false),
		}))
	}

	fn subscribe_logic_step(_node: &Node, calling_client: Arc<Client>, _data: &[u8]) -> Result<()> {
		calling_client
			.root
			.get()
			.unwrap()
			.logic_step
			.store(true, Ordering::Relaxed);
		Ok(())
	}

	#[instrument(level = "debug")]
	pub fn logic_step(delta: f64) {
		if let Ok(data) = serialize((delta, 0.0)) {
			for root in ROOT_REGISTRY.get_valid_contents() {
				if root.logic_step.load(Ordering::Relaxed) {
					let _ = root.node.send_remote_signal("logic_step", &data);
				}
			}
		}
	}

	fn set_base_prefixes(_node: &Node, calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		*calling_client.base_resource_prefixes.lock() = deserialize(data)?;
		Ok(())
	}
}

impl Drop for Root {
	fn drop(&mut self) {
		ROOT_REGISTRY.remove(self);
	}
}

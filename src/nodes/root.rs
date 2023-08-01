use super::spatial::Spatial;
use super::{Message, Node};
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
	send_frame_event: AtomicBool,
}
impl Root {
	pub fn create(client: &Arc<Client>) -> Result<Arc<Self>> {
		let node = Node::create(client, "", "", false);
		node.add_local_signal("subscribe_frame", Root::subscribe_frame_flex);
		node.add_local_signal("set_base_prefixes", Root::set_base_prefixes_flex);
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
			send_frame_event: AtomicBool::from(false),
		}))
	}

	fn subscribe_frame_flex(
		_node: &Node,
		calling_client: Arc<Client>,
		_message: Message,
	) -> Result<()> {
		calling_client
			.root
			.get()
			.unwrap()
			.send_frame_event
			.store(true, Ordering::Relaxed);
		Ok(())
	}

	#[instrument(level = "debug")]
	pub fn send_frame_events(delta: f64) {
		if let Ok(data) = serialize((delta, 0.0)) {
			for root in ROOT_REGISTRY.get_valid_contents() {
				if root.send_frame_event.load(Ordering::Relaxed) {
					let _ = root.node.send_remote_signal("frame", data.clone());
				}
			}
		}
	}

	fn set_base_prefixes_flex(
		_node: &Node,
		calling_client: Arc<Client>,
		message: Message,
	) -> Result<()> {
		*calling_client.base_resource_prefixes.lock() = deserialize(message.as_ref())?;
		Ok(())
	}
}

impl Drop for Root {
	fn drop(&mut self) {
		ROOT_REGISTRY.remove(self);
	}
}

use super::core::Node;
use super::spatial::Spatial;
use crate::core::client::Client;
use crate::core::registry::Registry;
use anyhow::Result;
use glam::Mat4;
use lazy_static::lazy_static;
use libstardustxr::flex::flexbuffer_from_vector_arguments;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

lazy_static! {
	static ref ROOT_REGISTRY: Registry<Root> = Default::default();
}

pub struct Root {
	node: Arc<Node>,
	logic_step: AtomicBool,
}
impl Root {
	pub fn create(client: &Arc<Client>) -> Arc<Self> {
		let node = Node::create(client, "", "", false);
		node.add_local_signal("subscribeLogicStep", Root::subscribe_logic_step);
		let node = node.add_to_scenegraph();
		let _ = Spatial::add_to(&node, None, Mat4::IDENTITY);

		ROOT_REGISTRY.add(Root {
			node,
			logic_step: AtomicBool::from(false),
		})
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

	pub fn logic_step(delta: f64) {
		let data = flexbuffer_from_vector_arguments(move |fbb| {
			fbb.push(delta);
			fbb.push(0_f64);
		});
		for root in ROOT_REGISTRY.get_valid_contents() {
			if root.logic_step.load(Ordering::Relaxed) {
				let _ = root.node.send_remote_signal("logicStep", &data);
			}
		}
	}
}

impl Drop for Root {
	fn drop(&mut self) {
		ROOT_REGISTRY.remove(self);
	}
}

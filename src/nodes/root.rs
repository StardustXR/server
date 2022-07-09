use super::core::Node;
use super::spatial::Spatial;
use crate::core::client::Client;
use crate::core::registry::Registry;
use anyhow::Result;
use glam::Mat4;
use lazy_static::lazy_static;
use libstardustxr::flex::flexbuffer_from_vector_arguments;
use std::sync::Arc;

lazy_static! {
	static ref LOGIC_STEP_REGISTRY: Registry<Node> = Registry::default();
}

pub fn logic_step(delta: f64) {
	let data = flexbuffer_from_vector_arguments(move |fbb| {
		fbb.push(delta);
		fbb.push(0_f64);
	});
	for root in LOGIC_STEP_REGISTRY.get_valid_contents() {
		root.send_remote_signal("logicStep", &data);
	}
}

pub fn create_root(client: &Arc<Client>) {
	let node = Node::create(client, "", "", false);
	node.add_local_signal("subscribeLogicStep", subscribe_logic_step);
	let node = node.add_to_scenegraph();
	let _ = Spatial::add_to(&node, None, Mat4::IDENTITY);
}

fn subscribe_logic_step(node: &Node, calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
	LOGIC_STEP_REGISTRY.add_raw(&calling_client.scenegraph.get_node("/").unwrap());
	Ok(())
}

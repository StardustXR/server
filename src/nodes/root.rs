use super::core::Node;
use super::spatial::Spatial;
use crate::core::client::Client;
use glam::Mat4;
use std::sync::Arc;

pub fn create_root(client: &Arc<Client>) {
	let node = Node::create("", "", false);
	let node = client.scenegraph.add_node(node);
	let _ = Spatial::add_to(&node, None, Mat4::IDENTITY);
}

use super::core::Node;
use super::spatial::Spatial;
use crate::core::client::Client;
use glam::Mat4;
use std::sync::Arc;

pub fn create_root(client: &Arc<Client>) {
	let node = Node::create(client, "", "", false).add_to_scenegraph();
	let _ = Spatial::add_to(&node, None, Mat4::IDENTITY);
}

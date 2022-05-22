use super::core::{Node, NodeData, NodeRef};
use crate::core::client::Client;
use anyhow::Result;
use vek::mat::repr_c::row_major::Mat4;

pub struct Spatial<'a> {
	node: &'a Node<'a>,
	transform: Mat4<f32>,
}

impl<'a> Spatial<'a> {
	pub fn new(node: &'a Node<'a>, transform: Mat4<f32>) -> Self {
		Spatial { node, transform }
	}

	pub fn new_node(
		client: Option<&mut Client<'a>>,
		path: &str,
		transform: Mat4<f32>,
	) -> Result<NodeRef<'a>> {
		let node = Node::from_path(client, path)?;
		node.upgrade().unwrap().borrow_mut().spatial = Some(Spatial::new(transform));
		Ok(node)
	}
}

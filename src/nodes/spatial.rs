use super::core::{Node, NodeData, NodeRef};
use crate::core::client::Client;
use anyhow::Result;
use vek::mat::repr_c::row_major::Mat4;

pub struct Spatial<'a> {
	node: NodeRef<'a>,
	transform: Mat4<f32>,
}

impl<'a> Spatial<'a> {
	pub fn new(node: NodeRef<'a>, transform: Mat4<f32>) -> Self {
		Spatial { node, transform }
	}

	pub fn new_node(
		client: Option<&mut Client<'a>>,
		path: &str,
		transform: Mat4<f32>,
	) -> Result<NodeRef<'a>> {
		Node::from_path(client, path, move |node| {
			NodeData::Spatial(Spatial::new(node, transform))
		})
	}
}

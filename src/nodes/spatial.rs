use super::core::Node;
use crate::core::client::Client;
use anyhow::Result;
use std::cell::RefCell;
use std::rc::Weak;
use vek::mat::repr_c::row_major::Mat4;

pub struct Spatial {
	transform: Mat4<f32>,
}

impl<'a> Spatial {
	pub fn new(transform: Mat4<f32>) -> Self {
		Spatial { transform }
	}

	pub fn new_node(
		client: Option<&'a Client<'a>>,
		path: &str,
		transform: Mat4<f32>,
	) -> Result<Weak<RefCell<Node<'a>>>> {
		let node = Node::from_path(client, path)?;
		node.upgrade().unwrap().borrow_mut().spatial = Some(Spatial::new(transform));
		Ok(node)
	}
}

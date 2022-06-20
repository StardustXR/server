use crate::nodes::core::Node;
use anyhow::{anyhow, ensure, Result};
use parking_lot::RwLock;
use std::sync::Weak;

#[derive(Default)]
pub struct LifeLinkedNodeList {
	nodes: RwLock<Vec<Weak<Node>>>,
}
impl LifeLinkedNodeList {
	pub fn add(&self, node: Weak<Node>) {
		self.nodes.write().push(node);
	}

	pub fn clear(&self) {
		self.nodes
			.read()
			.iter()
			.filter_map(|node| node.upgrade())
			.for_each(|node| {
				node.destroy();
			});
		self.nodes.write().clear();
	}
}
impl Drop for LifeLinkedNodeList {
	fn drop(&mut self) {
		self.clear();
	}
}

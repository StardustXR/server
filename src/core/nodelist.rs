use crate::nodes::Node;
use parking_lot::Mutex;
use std::sync::Weak;

#[derive(Default)]
pub struct LifeLinkedNodeList {
	nodes: Mutex<Vec<Weak<Node>>>,
}
impl LifeLinkedNodeList {
	pub fn add(&self, node: Weak<Node>) {
		self.nodes.lock().push(node);
	}

	pub fn clear(&self) {
		self.nodes
			.lock()
			.iter()
			.filter_map(|node| node.upgrade())
			.for_each(|node| {
				node.destroy();
			});
		self.nodes.lock().clear();
	}
}
impl Drop for LifeLinkedNodeList {
	fn drop(&mut self) {
		self.clear();
	}
}

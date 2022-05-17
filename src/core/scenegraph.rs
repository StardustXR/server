use crate::nodes::core::Node;
use anyhow::Result;
use libstardustxr::scenegraph;
use libstardustxr::scenegraph::ScenegraphError;
use std::{cell::RefCell, collections::HashMap, rc::Rc, rc::Weak};

#[derive(Default)]
pub struct Scenegraph<'a> {
	nodes: HashMap<String, Rc<RefCell<Node<'a>>>>,
}

impl<'a> Scenegraph<'a> {
	pub fn new() -> Self {
		Default::default()
	}

	pub fn add_node(&mut self, node: Rc<RefCell<Node<'a>>>) {
		let path = node.borrow().get_path().to_string();
		self.nodes.insert(path, node);
	}

	pub fn remove_node(&mut self, path: &str) {
		self.nodes.remove(path);
	}

	pub fn get_node(&self, path: &str) -> Weak<RefCell<Node<'a>>> {
		self.nodes
			.get(path)
			.map_or(Weak::default(), |node| Rc::downgrade(node))
	}
}

impl<'a> scenegraph::Scenegraph for Scenegraph<'a> {
	fn send_signal(&self, path: &str, method: &str, data: &[u8]) -> Result<(), ScenegraphError> {
		self.nodes
			.get(path)
			.ok_or(ScenegraphError::NodeNotFound)?
			.borrow()
			.send_local_signal(method, data)
			.map_err(|_| ScenegraphError::MethodNotFound)
	}
	fn execute_method(
		&self,
		path: &str,
		method: &str,
		data: &[u8],
	) -> Result<Vec<u8>, ScenegraphError> {
		self.nodes
			.get(path)
			.ok_or(ScenegraphError::NodeNotFound)?
			.borrow()
			.execute_local_method(method, data)
			.map_err(|_| ScenegraphError::MethodNotFound)
	}
}

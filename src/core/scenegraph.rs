use crate::nodes::core::Node;
use anyhow::Result;
use libstardustxr::scenegraph;
use libstardustxr::scenegraph::ScenegraphError;
use std::{cell::RefCell, collections::HashMap, rc::Weak};

#[derive(Default)]
pub struct Scenegraph<'a> {
	nodes: RefCell<HashMap<String, Weak<Node<'a>>>>,
}

impl<'a> Scenegraph<'a> {
	pub fn new() -> Self {
		Default::default()
	}

	pub fn add_node(&self, node: Weak<Node<'a>>) {
		let node_ref = node.upgrade();
		if node_ref.is_none() {
			return;
		}
		self.nodes
			.borrow_mut()
			.insert(String::from(node_ref.unwrap().get_path()), node);
	}

	pub fn remove_node(&self, node: Weak<Node<'a>>) {
		let node_ref = node.upgrade();
		if node_ref.is_none() {
			return;
		}
		self.nodes.borrow_mut().remove(node_ref.unwrap().get_path());
	}

	pub fn get_node(&self, path: &str) -> Weak<Node<'a>> {
		self.nodes.borrow().get(path).cloned().unwrap_or_default()
	}
}

impl<'a> scenegraph::Scenegraph for Scenegraph<'a> {
	fn send_signal(&self, path: &str, method: &str, data: &[u8]) -> Result<(), ScenegraphError> {
		self.nodes
			.borrow()
			.get(path)
			.ok_or(ScenegraphError::NodeNotFound)?
			.upgrade()
			.ok_or(ScenegraphError::NodeNotFound)?
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
			.borrow()
			.get(path)
			.ok_or(ScenegraphError::NodeNotFound)?
			.upgrade()
			.ok_or(ScenegraphError::NodeNotFound)?
			.execute_local_method(method, data)
			.map_err(|_| ScenegraphError::MethodNotFound)
	}
}

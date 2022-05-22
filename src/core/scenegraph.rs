use crate::core::client::Client;
use crate::nodes::core::{Node, NodeRef};
use crate::nodes::spatial::Spatial;
use anyhow::Result;
use libstardustxr::scenegraph;
use libstardustxr::scenegraph::ScenegraphError;
use rccell::{RcCell, WeakCell};
use std::collections::HashMap;

pub struct Scenegraph<'a> {
	nodes: HashMap<String, RcCell<Node<'a>>>,
	root: NodeRef<'a>,
	hmd: NodeRef<'a>,
}

impl<'a> Scenegraph<'a> {
	pub fn new(client: &mut Client<'a>) -> Self {
		Scenegraph {
			nodes: HashMap::new(),
			root: Spatial::new_node(Some(client), "/", Default::default()).unwrap(),
			hmd: Spatial::new_node(Some(client), "/hmd", Default::default()).unwrap(),
		}
	}
	pub fn add_node(&mut self, node: RcCell<Node<'a>>) {
		let path = node.borrow().get_path().to_string();
		self.nodes.insert(path, node);
	}

	pub fn remove_node(&mut self, path: &str) {
		self.nodes.remove(path);
	}

	pub fn get_node(&self, path: &str) -> WeakCell<Node<'a>> {
		self.nodes
			.get(path)
			.map_or(WeakCell::new(), RcCell::downgrade)
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

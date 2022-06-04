use crate::core::client::Client;
use crate::nodes::core::Node;
use anyhow::Result;
use libstardustxr::scenegraph;
use libstardustxr::scenegraph::ScenegraphError;
use rccell::{RcCell, WeakCell};
use std::collections::HashMap;

pub struct Scenegraph<'a> {
	client: WeakCell<Client<'a>>,
	pub nodes: HashMap<String, RcCell<Node<'a>>>,
}

impl<'a> Default for Scenegraph<'a> {
	fn default() -> Self {
		Scenegraph {
			client: WeakCell::new(),
			nodes: HashMap::new(),
		}
	}
}

impl<'a> scenegraph::Scenegraph for Scenegraph<'a> {
	fn send_signal(&self, path: &str, method: &str, data: &[u8]) -> Result<(), ScenegraphError> {
		self.nodes
			.get(path)
			.ok_or(ScenegraphError::NodeNotFound)?
			.borrow()
			.send_local_signal(self.client.upgrade().unwrap(), method, data)
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
			.execute_local_method(self.client.upgrade().unwrap(), method, data)
			.map_err(|_| ScenegraphError::MethodNotFound)
	}
}

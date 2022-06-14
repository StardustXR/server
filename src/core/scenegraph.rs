use crate::core::client::Client;
use crate::nodes::core::Node;
use anyhow::Result;
use libstardustxr::scenegraph;
use libstardustxr::scenegraph::ScenegraphError;
use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use core::hash::BuildHasherDefault;
use dashmap::DashMap;
use rustc_hash::FxHasher;

#[derive(Default)]
pub struct Scenegraph {
	client: RefCell<Weak<Client>>,
	nodes: DashMap<String, Arc<Node>, BuildHasherDefault<FxHasher>>,
}

impl Scenegraph {
	pub fn get_client(&self) -> Rc<Client> {
		self.client.borrow().upgrade().unwrap()
	}

	pub fn set_client(&self, client: &Rc<Client>) {
		*self.client.borrow_mut() = Rc::downgrade(client);
	}

	pub fn add_node(&self, node: Node) -> Arc<Node> {
		let mut node = node;
		node.client = Rc::downgrade(&self.get_client());
		let path = node.get_path().to_string();
		let node_arc = Arc::new(node);
		self.nodes.insert(path, node_arc.clone());
		node_arc
	}

	pub fn get_node(&self, path: &str) -> Option<Arc<Node>> {
		Some(self.nodes.get(path)?.clone())
	}

	pub fn remove_node(&self, path: &str) -> Option<Arc<Node>> {
		let (_, node) = self.nodes.remove(path)?;
		Some(node)
	}
}

impl scenegraph::Scenegraph for Scenegraph {
	fn send_signal(&self, path: &str, method: &str, data: &[u8]) -> Result<(), ScenegraphError> {
		self.get_node(path)
			.ok_or(ScenegraphError::NodeNotFound)?
			.send_local_signal(self.get_client(), method, data)
	}
	fn execute_method(
		&self,
		path: &str,
		method: &str,
		data: &[u8],
	) -> Result<Vec<u8>, ScenegraphError> {
		self.get_node(path)
			.ok_or(ScenegraphError::NodeNotFound)?
			.execute_local_method(self.get_client(), method, data)
	}
}

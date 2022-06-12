use crate::core::client::Client;
use crate::nodes::core::Node;
use anyhow::Result;
use core::hash::BuildHasherDefault;
use dashmap::DashMap;
use libstardustxr::scenegraph;
use libstardustxr::scenegraph::ScenegraphError;
use rccell::RcCell;
use rustc_hash::FxHasher;
use std::cell::RefCell;
use std::rc::{Rc, Weak};

#[derive(Default)]
pub struct Scenegraph<'a> {
	client: RefCell<Weak<Client<'a>>>,
	nodes: DashMap<String, RcCell<Node<'a>>, BuildHasherDefault<FxHasher>>,
}

impl<'a> Scenegraph<'a> {
	pub fn get_client(&self) -> Rc<Client<'a>> {
		self.client.borrow().upgrade().unwrap()
	}

	pub fn set_client(&self, client: &Rc<Client<'a>>) {
		*self.client.borrow_mut() = Rc::downgrade(client);
	}

	pub fn add_node(&self, node: Node<'a>) -> RcCell<Node<'a>> {
		let path = node.get_path().to_string();
		let node_rc = RcCell::new(node);
		self.nodes.insert(path, node_rc.clone());
		node_rc
	}

	pub fn get_node(&self, path: &str) -> Option<RcCell<Node<'a>>> {
		Some(self.nodes.get(path)?.clone())
	}

	pub fn remove_node(&self, path: &str) -> Option<RcCell<Node<'a>>> {
		let (_, node) = self.nodes.remove(path)?;
		Some(node)
	}
}

impl<'a> scenegraph::Scenegraph for Scenegraph<'a> {
	fn send_signal(&self, path: &str, method: &str, data: &[u8]) -> Result<(), ScenegraphError> {
		self.get_node(path)
			.ok_or(ScenegraphError::NodeNotFound)?
			.borrow()
			.send_local_signal(self.get_client(), method, data)
			.map_err(|_| ScenegraphError::MethodNotFound)
	}
	fn execute_method(
		&self,
		path: &str,
		method: &str,
		data: &[u8],
	) -> Result<Vec<u8>, ScenegraphError> {
		self.get_node(path)
			.ok_or(ScenegraphError::NodeNotFound)?
			.borrow()
			.execute_local_method(self.get_client(), method, data)
			.map_err(|_| ScenegraphError::MethodNotFound)
	}
}

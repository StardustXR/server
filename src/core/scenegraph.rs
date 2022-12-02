use crate::core::client::Client;
use crate::nodes::Node;
use color_eyre::eyre::Result;
use once_cell::sync::OnceCell;
use stardust_xr::scenegraph;
use stardust_xr::scenegraph::ScenegraphError;
use std::sync::{Arc, Weak};

use core::hash::BuildHasherDefault;
use dashmap::DashMap;
use rustc_hash::FxHasher;

#[derive(Default)]
pub struct Scenegraph {
	pub(super) client: OnceCell<Weak<Client>>,
	nodes: DashMap<String, Arc<Node>, BuildHasherDefault<FxHasher>>,
}

impl Scenegraph {
	pub fn get_client(&self) -> Arc<Client> {
		self.client.get().unwrap().upgrade().unwrap()
	}

	pub fn add_node(&self, node: Node) -> Arc<Node> {
		let node_arc = Arc::new(node);
		self.add_node_raw(node_arc.clone());
		node_arc
	}
	pub fn add_node_raw(&self, node: Arc<Node>) {
		let path = node.get_path().to_string();
		self.nodes.insert(path, node);
	}

	pub fn get_node(&self, path: &str) -> Option<Arc<Node>> {
		let mut node = self.nodes.get(path)?.clone();
		if let Some(alias) = node.alias.get() {
			node = alias.original.upgrade()?;
		}
		Some(node)
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

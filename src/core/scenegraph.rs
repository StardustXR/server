use crate::core::client::Client;
use crate::nodes::spatial::Spatial;
use anyhow::Result;
use libstardustxr::scenegraph;
use libstardustxr::scenegraph::ScenegraphError;
use rccell::{RcCell, WeakCell};
use std::collections::HashMap;

pub struct Scenegraph<'a> {
	client: WeakCell<Client<'a>>,
	pub spatial_nodes: HashMap<String, RcCell<Spatial<'a>>>,
}

impl<'a> Scenegraph<'a> {
	pub fn new(client: RcCell<Client<'a>>) -> Self {
		// root: Spatial::new(Some(client), "/", Default::default()),
		// hmd: Spatial::new(Some(client), "/hmd", Default::default()),
		Scenegraph {
			client: client.downgrade(),
			spatial_nodes: HashMap::new(),
		}
	}
}

impl<'a> Default for Scenegraph<'a> {
	fn default() -> Self {
		Scenegraph {
			client: WeakCell::new(),
			spatial_nodes: HashMap::new(),
		}
	}
}

impl<'a> scenegraph::Scenegraph for Scenegraph<'a> {
	fn send_signal(&self, path: &str, method: &str, data: &[u8]) -> Result<(), ScenegraphError> {
		self.spatial_nodes
			.get(path)
			.ok_or(ScenegraphError::NodeNotFound)?
			.borrow()
			.node
			.send_local_signal(self.client.upgrade().unwrap(), method, data)
			.map_err(|_| ScenegraphError::MethodNotFound)
	}
	fn execute_method(
		&self,
		path: &str,
		method: &str,
		data: &[u8],
	) -> Result<Vec<u8>, ScenegraphError> {
		self.spatial_nodes
			.get(path)
			.ok_or(ScenegraphError::NodeNotFound)?
			.borrow()
			.node
			.execute_local_method(self.client.upgrade().unwrap(), method, data)
			.map_err(|_| ScenegraphError::MethodNotFound)
	}
}

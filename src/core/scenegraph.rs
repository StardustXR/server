use crate::core::client::Client;
use crate::nodes::spatial::Spatial;
use anyhow::Result;
use libstardustxr::scenegraph;
use libstardustxr::scenegraph::ScenegraphError;
use rccell::RcCell;
use std::collections::HashMap;

pub struct Scenegraph<'a> {
	pub spatial_nodes: HashMap<String, RcCell<Spatial<'a>>>,
}

impl<'a> Scenegraph<'a> {
	pub fn new(client: RcCell<Client<'a>>) -> Self {
		// root: Spatial::new(Some(client), "/", Default::default()),
		// hmd: Spatial::new(Some(client), "/hmd", Default::default()),
		Scenegraph {
			spatial_nodes: HashMap::new(),
		}
	}
}

impl<'a> Default for Scenegraph<'a> {
	fn default() -> Self {
		Scenegraph {
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
			.send_local_signal(method, data)
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
			.execute_local_method(method, data)
			.map_err(|_| ScenegraphError::MethodNotFound)
	}
}

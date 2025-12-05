use crate::{
	core::{Id, client::Client, error::Result},
	nodes::{Message, Node, alias::get_original},
};
use dashmap::DashMap;
use stardust_xr_wire::{
	messenger::MethodResponse,
	scenegraph::{self, ScenegraphError},
};
use std::{
	os::fd::OwnedFd,
	sync::{Arc, OnceLock, Weak},
};
use tracing::{debug, debug_span};

#[derive(Default)]
pub struct Scenegraph {
	pub(super) client: OnceLock<Weak<Client>>,
	nodes: DashMap<Id, Arc<Node>, rustc_hash::FxBuildHasher>,
}
impl Scenegraph {
	pub fn get_client(&self) -> Option<Arc<Client>> {
		self.client.get()?.upgrade()
	}

	pub fn add_node(&self, node: Node) -> Arc<Node> {
		let node_arc = Arc::new(node);
		self.add_node_raw(node_arc.clone());
		node_arc
	}
	pub fn add_node_raw(&self, node: Arc<Node>) {
		debug!(node = ?&*node, "Add node");
		self.nodes.insert(node.get_id(), node);
	}

	pub fn get_node(&self, node: Id) -> Option<Arc<Node>> {
		let node = self.nodes.get(&node)?.clone();
		get_original(node, true)
	}

	pub fn remove_node(&self, node: Id) -> Option<Arc<Node>> {
		debug!(node = node.0, "Remove node");
		self.nodes.remove(&node).map(|(_, node)| node)
	}
}
impl scenegraph::Scenegraph for Scenegraph {
	fn send_signal(
		&self,
		node_id: u64,
		aspect_id: u64,
		method: u64,
		data: &[u8],
		fds: Vec<OwnedFd>,
	) -> Result<(), ScenegraphError> {
		let Some(client) = self.get_client() else {
			return Err(ScenegraphError::NodeNotFound);
		};
		debug_span!("Handle signal", aspect_id, node_id, method).in_scope(|| {
			self.get_node(Id(node_id))
				.ok_or(ScenegraphError::NodeNotFound)?
				.send_local_signal(
					client,
					aspect_id,
					method,
					Message {
						data: data.to_vec(),
						fds,
					},
				)
		})
	}
	fn execute_method(
		&self,
		node_id: u64,
		aspect_id: u64,
		method: u64,
		data: &[u8],
		fds: Vec<OwnedFd>,
		response: MethodResponse,
	) {
		let Some(client) = self.get_client() else {
			response.send(Err(ScenegraphError::NodeNotFound));
			return;
		};
		debug!(aspect_id, node_id, method, "Handle method");
		let Some(node) = self.get_node(Id(node_id)) else {
			response.send(Err(ScenegraphError::NodeNotFound));
			return;
		};
		node.execute_local_method(
			client,
			aspect_id,
			method,
			Message {
				data: data.to_vec(),
				fds,
			},
			response.into(),
		);
	}
}

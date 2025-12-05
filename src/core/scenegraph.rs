use crate::{
	core::{
		Id,
		client::Client,
		error::{Result, ServerError},
	},
	nodes::{Message, Node, alias::get_original},
};
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use serde::Serialize;
use stardust_xr_wire::{
	flex::serialize,
	messenger::MethodResponse,
	scenegraph::{self, ScenegraphError},
};
use std::{
	os::fd::OwnedFd,
	sync::{Arc, OnceLock, Weak},
};
use tracing::{debug, debug_span};

pub struct MethodResponseSender(pub(crate) MethodResponse);
impl MethodResponseSender {
	pub fn send_err(self, error: ScenegraphError) {
		self.0.send(Err(error));
	}
	pub fn send<T: Serialize>(self, result: Result<T, ServerError>) {
		let data = match result {
			Ok(d) => d,
			Err(e) => {
				self.0.send(Err(ScenegraphError::MemberError {
					error: e.to_string(),
				}));
				return;
			}
		};
		let Ok(serialized) = stardust_xr_wire::flex::serialize(data) else {
			self.0.send(Err(ScenegraphError::MemberError {
				error: "Internal: Failed to serialize".to_string(),
			}));
			return;
		};
		self.0.send(Ok((&serialized, Vec::<OwnedFd>::new())));
	}
	pub fn wrap<T: Serialize, F: FnOnce() -> Result<T>>(self, f: F) {
		self.send(f())
	}
	pub fn wrap_async<T: Serialize>(
		self,
		f: impl Future<Output = Result<(T, Vec<OwnedFd>)>> + Send + 'static,
	) {
		tokio::task::spawn(async move {
			let (value, fds) = match f.await {
				Ok(d) => d,
				Err(e) => {
					self.0.send(Err(ScenegraphError::MemberError {
						error: e.to_string(),
					}));
					return;
				}
			};
			let Ok(serialized) = serialize(value) else {
				self.0.send(Err(ScenegraphError::MemberError {
					error: "Internal: Failed to serialize".to_string(),
				}));
				return;
			};
			self.0.send(Ok((&serialized, fds)));
		});
	}
}
impl std::fmt::Debug for MethodResponseSender {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("TypedMethodResponse").finish()
	}
}

#[derive(Default)]
pub struct Scenegraph {
	pub(super) client: OnceLock<Weak<Client>>,
	nodes: Mutex<FxHashMap<Id, Arc<Node>>>,
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
		self.nodes.lock().insert(node.get_id(), node);
	}

	pub fn get_node(&self, node: Id) -> Option<Arc<Node>> {
		let node = self.nodes.lock().get(&node)?.clone();
		get_original(node, true)
	}

	pub fn remove_node(&self, node: Id) -> Option<Arc<Node>> {
		debug!(node = node.0, "Remove node");
		self.nodes.lock().remove(&node)
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
			MethodResponseSender(response),
		);
	}
}

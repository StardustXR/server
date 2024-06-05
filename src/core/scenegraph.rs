use crate::nodes::alias::Alias;
use crate::nodes::Node;
use crate::{core::client::Client, nodes::Message};
use color_eyre::eyre::Result;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use portable_atomic::Ordering;
use rustc_hash::FxHashMap;
use serde::Serialize;
use stardust_xr::scenegraph;
use stardust_xr::scenegraph::ScenegraphError;
use stardust_xr::schemas::flex::serialize;
use std::future::Future;
use std::os::fd::OwnedFd;
use std::sync::{Arc, Weak};
use tokio::sync::oneshot;
use tracing::{debug, debug_span};

#[derive(Default)]
pub struct Scenegraph {
	pub(super) client: OnceCell<Weak<Client>>,
	nodes: Mutex<FxHashMap<u64, Arc<Node>>>,
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

	pub fn get_node(&self, node: u64) -> Option<Arc<Node>> {
		let mut node = self.nodes.lock().get(&node)?.clone();
		while let Ok(alias) = node.get_aspect::<Alias>() {
			if alias.enabled.load(Ordering::Acquire) {
				node = alias.original.upgrade()?;
			} else {
				return None;
			}
		}
		Some(node)
	}

	pub fn remove_node(&self, node: u64) -> Option<Arc<Node>> {
		debug!(node, "Remove node");
		self.nodes.lock().remove(&node)
	}
}

pub struct MethodResponseSender(oneshot::Sender<Result<(Vec<u8>, Vec<OwnedFd>), ScenegraphError>>);
impl MethodResponseSender {
	pub fn send(self, t: Result<Message, ScenegraphError>) {
		let _ = self.0.send(t.map(|m| (m.data, m.fds)));
	}
	// pub fn send_method_return<T: Serialize>(
	// 	self,
	// 	result: color_eyre::eyre::Result<(T, Vec<OwnedFd>)>,
	// ) {
	// 	let _ = self.0.send(map_method_return(result));
	// }
	pub fn wrap_sync<F: FnOnce() -> color_eyre::eyre::Result<Message>>(self, f: F) {
		self.send(f().map_err(|e| ScenegraphError::MethodError {
			error: e.to_string(),
		}))
	}
	pub fn wrap_async<T: Serialize>(
		self,
		f: impl Future<Output = color_eyre::eyre::Result<(T, Vec<OwnedFd>)>> + Send + 'static,
	) {
		tokio::task::spawn(async move { self.0.send(map_method_return(f.await)) });
	}
}
fn map_method_return<T: Serialize>(
	result: color_eyre::eyre::Result<(T, Vec<OwnedFd>)>,
) -> Result<(Vec<u8>, Vec<OwnedFd>), ScenegraphError> {
	let (value, fds) = result.map_err(|e| ScenegraphError::MethodError {
		error: e.to_string(),
	})?;

	let serialized_value = serialize(value).map_err(|e| ScenegraphError::MethodError {
		error: format!("Internal: Serialization failed: {e}"),
	})?;
	Ok((serialized_value, fds))
}
impl scenegraph::Scenegraph for Scenegraph {
	fn send_signal(
		&self,
		node: u64,
		method: u64,
		data: &[u8],
		fds: Vec<OwnedFd>,
	) -> Result<(), ScenegraphError> {
		let Some(client) = self.get_client() else {
			return Err(ScenegraphError::SignalNotFound);
		};
		debug_span!("Handle signal", node, method).in_scope(|| {
			self.get_node(node)
				.ok_or(ScenegraphError::NodeNotFound)?
				.send_local_signal(
					client,
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
		node: u64,
		method: u64,
		data: &[u8],
		fds: Vec<OwnedFd>,
		response: oneshot::Sender<Result<(Vec<u8>, Vec<OwnedFd>), ScenegraphError>>,
	) {
		let Some(client) = self.get_client() else {
			let _ = response.send(Err(ScenegraphError::MethodNotFound));
			return;
		};
		debug!(node, method, "Handle method");
		let Some(node) = self.get_node(node) else {
			let _ = response.send(Err(ScenegraphError::NodeNotFound));
			return;
		};
		node.execute_local_method(
			client,
			method,
			Message {
				data: data.to_vec(),
				fds,
			},
			MethodResponseSender(response),
		);
	}
}

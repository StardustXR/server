use crate::nodes::Node;
use crate::{core::client::Client, nodes::Message};
use color_eyre::eyre::Result;
use once_cell::sync::OnceCell;
use stardust_xr::scenegraph;
use stardust_xr::scenegraph::ScenegraphError;
use std::os::fd::OwnedFd;
use std::sync::{Arc, Weak};
use tokio::sync::oneshot;
use tracing::{debug, debug_span, instrument};

use core::hash::BuildHasherDefault;
use dashmap::DashMap;
use rustc_hash::FxHasher;

#[derive(Default)]
pub struct Scenegraph {
	pub(super) client: OnceCell<Weak<Client>>,
	nodes: DashMap<String, Arc<Node>, BuildHasherDefault<FxHasher>>,
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
		let path = node.get_path().to_string();
		self.nodes.insert(path, node);
	}

	#[instrument(level = "debug", skip(self))]
	pub fn get_node(&self, path: &str) -> Option<Arc<Node>> {
		let mut node = self.nodes.get(path)?.clone();
		while let Some(alias) = node.alias.get() {
			node = alias.original.upgrade()?;
		}
		Some(node)
	}

	pub fn remove_node(&self, path: &str) -> Option<Arc<Node>> {
		debug!(path, "Remove node");
		let (_, node) = self.nodes.remove(path)?;
		Some(node)
	}
}

pub struct MethodResponseSender(oneshot::Sender<Result<(Vec<u8>, Vec<OwnedFd>), ScenegraphError>>);
impl MethodResponseSender {
	pub fn send(self, t: Result<Message, ScenegraphError>) {
		let _ = self.0.send(t.map(|m| (m.data, m.fds)));
	}
	pub fn wrap_sync<F: FnOnce() -> color_eyre::eyre::Result<Message>>(self, f: F) {
		self.send(f().map_err(|e| ScenegraphError::MethodError {
			error: e.to_string(),
		}))
	}
}
impl scenegraph::Scenegraph for Scenegraph {
	fn send_signal(
		&self,
		path: &str,
		method: &str,
		data: &[u8],
		fds: Vec<OwnedFd>,
	) -> Result<(), ScenegraphError> {
		let Some(client) = self.get_client() else {
			return Err(ScenegraphError::SignalNotFound);
		};
		debug_span!("Handle signal", path, method).in_scope(|| {
			self.get_node(path)
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
		path: &str,
		method: &str,
		data: &[u8],
		fds: Vec<OwnedFd>,
		response: oneshot::Sender<Result<(Vec<u8>, Vec<OwnedFd>), ScenegraphError>>,
	) {
		let Some(client) = self.get_client() else {
			let _ = response.send(Err(ScenegraphError::MethodNotFound));
			return;
		};
		debug!(path, method, "Handle method");
		let Some(node) = self.get_node(path) else {
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

use super::data::{PulseReceiver, PulseSender};
use super::field::Field;
use super::spatial::Spatial;
use crate::core::client::Client;
use anyhow::{anyhow, Result};
use libstardustxr::scenegraph::ScenegraphError;
use nanoid::nanoid;
use once_cell::sync::OnceCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::vec::Vec;

use core::hash::BuildHasherDefault;
use dashmap::DashMap;
use rustc_hash::FxHasher;

pub type Signal = fn(&Node, Arc<Client>, &[u8]) -> Result<()>;
pub type Method = fn(&Node, Arc<Client>, &[u8]) -> Result<Vec<u8>>;

pub struct Node {
	pub(super) uid: String,
	pub(crate) client: Weak<Client>,
	path: String,
	// trailing_slash_pos: usize,
	local_signals: DashMap<String, Signal, BuildHasherDefault<FxHasher>>,
	local_methods: DashMap<String, Method, BuildHasherDefault<FxHasher>>,
	destroyable: AtomicBool,

	alias: OnceCell<Alias>,
	pub spatial: OnceCell<Arc<Spatial>>,
	pub field: OnceCell<Arc<Field>>,
	pub pulse_sender: OnceCell<Arc<PulseSender>>,
	pub pulse_receiver: OnceCell<Arc<PulseReceiver>>,
}

impl Node {
	pub fn get_client(&self) -> Option<Arc<Client>> {
		self.client.clone().upgrade()
	}
	// pub fn get_name(&self) -> &str {
	// 	&self.path[self.trailing_slash_pos + 1..]
	// }
	pub fn get_path(&self) -> &str {
		self.path.as_str()
	}
	pub fn is_destroyable(&self) -> bool {
		self.destroyable.load(Ordering::Relaxed)
	}

	pub fn create(parent: &str, name: &str, destroyable: bool) -> Self {
		let mut path = parent.to_string();
		path.push('/');
		path.push_str(name);
		let node = Node {
			uid: nanoid!(),
			client: Weak::new(),
			path,
			// trailing_slash_pos: parent.len(),
			local_signals: Default::default(),
			local_methods: Default::default(),
			destroyable: AtomicBool::from(destroyable),

			alias: OnceCell::new(),
			spatial: OnceCell::new(),
			field: OnceCell::new(),
			pulse_sender: OnceCell::new(),
			pulse_receiver: OnceCell::new(),
		};
		node.add_local_signal("destroy", Node::destroy_flex);
		node
	}
	pub fn destroy(&self) {
		if let Some(client) = self.get_client() {
			let _ = client.scenegraph.remove_node(self.get_path());
		}
	}

	pub fn destroy_flex(node: &Node, _calling_client: Arc<Client>, _data: &[u8]) -> Result<()> {
		if node.is_destroyable() {
			node.destroy();
		}
		Ok(())
	}

	pub fn add_local_signal(&self, name: &str, signal: Signal) {
		self.local_signals.insert(name.to_string(), signal);
	}
	pub fn add_local_method(&self, name: &str, method: Method) {
		self.local_methods.insert(name.to_string(), method);
	}

	pub fn send_local_signal(
		&self,
		calling_client: Arc<Client>,
		method: &str,
		data: &[u8],
	) -> Result<(), ScenegraphError> {
		if let Some(alias) = self.alias.get().as_ref() {
			if !alias.signals.iter().any(|e| e == &method) {
				return Err(ScenegraphError::SignalNotFound);
			}
			alias
				.original
				.upgrade()
				.ok_or(ScenegraphError::BrokenAlias)?
				.send_local_signal(calling_client, method, data)
		} else {
			let signal = self
				.local_signals
				.get(method)
				.ok_or(ScenegraphError::SignalNotFound)?;
			signal(self, calling_client, data)
				.map_err(|error| ScenegraphError::SignalError { error })
		}
	}
	pub fn execute_local_method(
		&self,
		calling_client: Arc<Client>,
		method: &str,
		data: &[u8],
	) -> Result<Vec<u8>, ScenegraphError> {
		if let Some(alias) = self.alias.get().as_ref() {
			if !alias.methods.iter().any(|e| e == &method) {
				return Err(ScenegraphError::MethodNotFound);
			}
			alias
				.original
				.upgrade()
				.ok_or(ScenegraphError::BrokenAlias)?
				.execute_local_method(calling_client, method, data)
		} else {
			let method = self
				.local_methods
				.get(method)
				.ok_or(ScenegraphError::MethodNotFound)?;
			method(self, calling_client, data)
				.map_err(|error| ScenegraphError::MethodError { error })
		}
	}
	pub fn send_remote_signal(&self, method: &str, data: &[u8]) -> Result<()> {
		self.get_client()
			.ok_or_else(|| anyhow!("Node has no client, can't send remote signal!"))?
			.messenger
			.send_remote_signal(self.path.as_str(), method, data)
			.map_err(|_| anyhow!("Unable to write in messenger"))
	}
	// pub fn execute_remote_method(
	// 	&self,
	// 	method: &str,
	// 	data: &[u8],
	// 	callback: Box<dyn Fn(&[u8]) + 'a>,
	// ) -> Result<()> {
	// 	self.get_client()
	// 		.ok_or_else(|| anyhow!("Node has no client, can't send remote signal!"))?
	// 		.messenger
	// 		.execute_remote_method(self.path.as_str(), method, data, callback)
	// 		.map_err(|_| anyhow!("Unable to write in messenger"))
	// }
}

pub struct Alias {
	original: Weak<Node>,

	signals: Vec<&'static str>,
	methods: Vec<&'static str>,
}
impl Alias {
	pub fn add_to(
		node: &Arc<Node>,
		original: &Arc<Node>,
		signals: Vec<&'static str>,
		methods: Vec<&'static str>,
	) {
		let _ = node.alias.set(Alias {
			original: Arc::downgrade(original),
			signals,
			methods,
		});
	}
}

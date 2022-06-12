use super::data::PulseSender;
use super::field::Field;
use super::spatial::Spatial;
use crate::core::client::Client;
use anyhow::Result;
use libstardustxr::scenegraph::ScenegraphError;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::{collections::HashMap, vec::Vec};

pub type Signal = fn(&Node, Rc<Client>, &[u8]) -> Result<()>;
pub type Method = fn(&Node, Rc<Client>, &[u8]) -> Result<Vec<u8>>;

pub struct Node<'a> {
	client: Weak<Client<'a>>,
	path: String,
	// trailing_slash_pos: usize,
	local_signals: HashMap<String, Signal>,
	local_methods: HashMap<String, Method>,
	destroyable: bool,

	pub spatial: Option<Rc<Spatial>>,
	pub field: Option<Rc<Field>>,
	pub pulse_sender: Option<Arc<PulseSender>>,
}

impl<'a> Node<'a> {
	pub fn get_client(&self) -> Option<Rc<Client<'a>>> {
		self.client.clone().upgrade()
	}
	// pub fn get_name(&self) -> &str {
	// 	&self.path[self.trailing_slash_pos + 1..]
	// }
	pub fn get_path(&self) -> &str {
		self.path.as_str()
	}
	pub fn is_destroyable(&self) -> bool {
		self.destroyable
	}

	pub fn create(client: Weak<Client<'a>>, parent: &str, name: &str, destroyable: bool) -> Self {
		let mut path = parent.to_string();
		path.push('/');
		path.push_str(name);
		let mut node = Node {
			client,
			path,
			// trailing_slash_pos: parent.len(),
			local_signals: HashMap::new(),
			local_methods: HashMap::new(),
			destroyable,

			spatial: None,
			field: None,
			pulse_sender: None,
		};
		node.add_local_signal("destroy", Node::destroy_flex);
		node
	}
	pub fn destroy(&self) {
		if let Some(client) = self.get_client() {
			let _ = client.get_scenegraph().remove_node(self.get_path());
		}
	}

	pub fn destroy_flex(node: &Node, _calling_client: Rc<Client>, _data: &[u8]) -> Result<()> {
		if node.is_destroyable() {
			node.destroy();
		}
		Ok(())
	}

	pub fn add_local_signal(&mut self, name: &str, signal: Signal) {
		self.local_signals.insert(name.to_string(), signal);
	}
	pub fn add_local_method(&mut self, name: &str, method: Method) {
		self.local_methods.insert(name.to_string(), method);
	}

	pub fn send_local_signal(
		&self,
		calling_client: Rc<Client>,
		method: &str,
		data: &[u8],
	) -> Result<(), ScenegraphError> {
		let signal = self
			.local_signals
			.get(method)
			.ok_or(ScenegraphError::SignalNotFound)?;
		signal(self, calling_client, data).map_err(|error| ScenegraphError::SignalError { error })
	}
	pub fn execute_local_method(
		&self,
		calling_client: Rc<Client>,
		method: &str,
		data: &[u8],
	) -> Result<Vec<u8>, ScenegraphError> {
		let method = self
			.local_methods
			.get(method)
			.ok_or(ScenegraphError::MethodNotFound)?;
		method(self, calling_client, data).map_err(|error| ScenegraphError::MethodError { error })
	}
	// pub fn send_remote_signal(&self, method: &str, data: &[u8]) -> Result<()> {
	// 	self.get_client()
	// 		.ok_or_else(|| anyhow!("Node has no client, can't send remote signal!"))?
	// 		.get_messenger()
	// 		.send_remote_signal(self.path.as_str(), method, data)
	// 		.map_err(|_| anyhow!("Unable to write in messenger"))
	// }
	// pub fn execute_remote_method(
	// 	&self,
	// 	method: &str,
	// 	data: &[u8],
	// 	callback: Box<dyn Fn(&[u8]) + 'a>,
	// ) -> Result<()> {
	// 	self.get_client()
	// 		.ok_or_else(|| anyhow!("Node has no client, can't send remote signal!"))?
	// 		.get_messenger()
	// 		.execute_remote_method(self.path.as_str(), method, data, callback)
	// 		.map_err(|_| anyhow!("Unable to write in messenger"))
	// }
}

use crate::core::client::Client;
use anyhow::{anyhow, ensure, Result};
use libstardustxr::messenger::Messenger;
use std::{
	collections::HashMap,
	rc::{Rc, Weak},
	vec::Vec,
};

type Signal<'a> = dyn Fn(&[u8]) + 'a;
type Method<'a> = dyn Fn(&[u8]) -> Vec<u8> + 'a;

pub struct Node<'a> {
	path: String,
	trailing_slash_pos: usize,
	pub messenger: Weak<Messenger<'a>>,
	local_signals: HashMap<String, Box<Signal<'a>>>,
	local_methods: HashMap<String, Box<Method<'a>>>,
}

impl<'a> Node<'a> {
	pub fn get_name(&self) -> &str {
		&self.path[self.trailing_slash_pos + 1..]
	}
	pub fn get_path(&self) -> &str {
		self.path.as_str()
	}

	pub fn from_path(client: &Client<'a>, path: &str) -> Result<Rc<Self>> {
		ensure!(path.starts_with('/'), "Invalid path {}", path);
		let node = Node {
			path: path.to_string(),
			trailing_slash_pos: path.rfind('/').ok_or(anyhow!("Invalid path {}", path))?,
			messenger: client.get_weak_messenger(),
			local_signals: HashMap::new(),
			local_methods: HashMap::new(),
		};
		let node_ref = Rc::new(node);
		client.scenegraph.add_node(Rc::downgrade(&node_ref));
		Ok(node_ref)
	}

	pub fn send_local_signal(&self, method: &str, data: &[u8]) -> Result<()> {
		self.local_signals
			.get(method)
			.ok_or(anyhow!("Signal {} not found", method))?(data);
		Ok(())
	}
	pub fn execute_local_method(&self, method: &str, data: &[u8]) -> Result<Vec<u8>> {
		Ok(self
			.local_methods
			.get(method)
			.ok_or(anyhow!("Method {} not found", method))?(data))
	}
	pub fn send_remote_signal(&self, method: &str, data: &[u8]) -> Result<()> {
		self.messenger
			.upgrade()
			.ok_or(anyhow!("Invalid messenger"))?
			.send_remote_signal(self.path.as_str(), method, data)
			.map_err(|_| anyhow!("Unable to write in messenger"))
	}
	pub fn execute_remote_method(
		&self,
		method: &str,
		data: &[u8],
		callback: Box<dyn Fn(&[u8]) + 'a>,
	) -> Result<()> {
		self.messenger
			.upgrade()
			.ok_or(anyhow!("Invalid messenger"))?
			.execute_remote_method(self.path.as_str(), method, data, callback)
			.map_err(|_| anyhow!("Unable to write in messenger"))
	}
}

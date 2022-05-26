use crate::core::client::Client;
use crate::core::scenegraph::Scenegraph;
use anyhow::{anyhow, ensure, Result};
use libstardustxr::messenger::Messenger;
use std::{collections::HashMap, rc::Weak, vec::Vec};

pub type Signal<'a> = Box<dyn Fn(&[u8]) -> Result<()> + 'a>;
pub type Method<'a> = Box<dyn Fn(&[u8]) -> Result<Vec<u8>> + 'a>;

pub struct Node<'a> {
	path: String,
	trailing_slash_pos: usize,
	messenger: Weak<Messenger<'a>>,
	scenegraph: Option<&'a Scenegraph<'a>>,
	local_signals: HashMap<String, Signal<'a>>,
	local_methods: HashMap<String, Method<'a>>,
}

impl<'a> Node<'a> {
	pub fn get_name(&self) -> &str {
		&self.path[self.trailing_slash_pos + 1..]
	}
	pub fn get_path(&self) -> &str {
		self.path.as_str()
	}

	pub fn from_path(
		client: Option<&Client<'a>>,
		path: &str,
		destroyable: bool,
	) -> Result<Node<'a>> {
		ensure!(path.starts_with('/'), "Invalid path {}", path);
		let mut weak_messenger = Weak::default();
		if let Some(c) = client.as_ref() {
			weak_messenger = c.get_weak_messenger();
		}
		Ok(Node {
			path: path.to_string(),
			trailing_slash_pos: path
				.rfind('/')
				.ok_or_else(|| anyhow!("Invalid path {}", path))?,
			messenger: weak_messenger,
			scenegraph: client.as_ref().map(|f| f.scenegraph.as_ref().unwrap()),
			local_signals: HashMap::new(),
			local_methods: HashMap::new(),
		})
	}

	pub fn add_local_signal(&mut self, method: &str, signal: Signal<'a>) {
		self.local_signals.insert(method.to_string(), signal);
	}

	pub fn send_local_signal(&self, method: &str, data: &[u8]) -> Result<()> {
		self.local_signals
			.get(method)
			.ok_or_else(|| anyhow!("Signal {} not found", method))?(data)
	}
	pub fn execute_local_method(&self, method: &str, data: &[u8]) -> Result<Vec<u8>> {
		self.local_methods
			.get(method)
			.ok_or_else(|| anyhow!("Method {} not found", method))?(data)
	}
	pub fn send_remote_signal(&self, method: &str, data: &[u8]) -> Result<()> {
		self.messenger
			.upgrade()
			.ok_or_else(|| anyhow!("Invalid messenger"))?
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
			.ok_or_else(|| anyhow!("Invalid messenger"))?
			.execute_remote_method(self.path.as_str(), method, data, callback)
			.map_err(|_| anyhow!("Unable to write in messenger"))
	}
}

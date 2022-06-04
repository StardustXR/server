use crate::core::client::Client;
use crate::nodes::spatial::Spatial;
use anyhow::{anyhow, ensure, Result};
use rccell::{RcCell, WeakCell};
use std::{collections::HashMap, vec::Vec};

pub type Signal<'a> = Box<dyn Fn(RcCell<Client>, &[u8]) -> Result<()> + 'a>;
pub type Method<'a> = Box<dyn Fn(RcCell<Client>, &[u8]) -> Result<Vec<u8>> + 'a>;

pub struct Node<'a> {
	client: WeakCell<Client<'a>>,
	path: String,
	trailing_slash_pos: usize,
	local_signals: HashMap<String, Signal<'a>>,
	local_methods: HashMap<String, Method<'a>>,
	destroyable: bool,

	pub spatial: Option<Spatial<'a>>,
}

impl<'a> Node<'a> {
	pub fn get_client(&self) -> Option<RcCell<Client<'a>>> {
		self.client.clone().upgrade()
	}
	pub fn get_name(&self) -> &str {
		&self.path[self.trailing_slash_pos + 1..]
	}
	pub fn get_path(&self) -> &str {
		self.path.as_str()
	}

	pub fn from_path(
		client: WeakCell<Client<'a>>,
		path: &str,
		destroyable: bool,
	) -> Result<Node<'a>> {
		ensure!(path.starts_with('/'), "Invalid path {}", path);
		Ok(Node {
			client,
			path: path.to_string(),
			trailing_slash_pos: path
				.rfind('/')
				.ok_or_else(|| anyhow!("Invalid path {}", path))?,
			local_signals: HashMap::new(),
			local_methods: HashMap::new(),
			destroyable,
			spatial: None,
		})
	}

	pub fn add_local_signal(&mut self, method: &str, signal: Signal<'a>) {
		self.local_signals.insert(method.to_string(), signal);
	}

	pub fn send_local_signal(
		&self,
		calling_client: RcCell<Client>,
		method: &str,
		data: &[u8],
	) -> Result<()> {
		let signal = self
			.local_signals
			.get(method)
			.ok_or_else(|| anyhow!("Signal {} not found", method))?;
		signal(calling_client, data)
	}
	pub fn execute_local_method(
		&self,
		calling_client: RcCell<Client>,
		method: &str,
		data: &[u8],
	) -> Result<Vec<u8>> {
		self.local_methods
			.get(method)
			.ok_or_else(|| anyhow!("Method {} not found", method))?(calling_client, data)
	}
	pub fn send_remote_signal(&self, method: &str, data: &[u8]) -> Result<()> {
		self.get_client()
			.ok_or(anyhow!("Node has no client, can't send remote signal!"))?
			.borrow()
			.get_messenger()
			.send_remote_signal(self.path.as_str(), method, data)
			.map_err(|_| anyhow!("Unable to write in messenger"))
	}
	pub fn execute_remote_method(
		&self,
		method: &str,
		data: &[u8],
		callback: Box<dyn Fn(&[u8]) + 'a>,
	) -> Result<()> {
		self.get_client()
			.ok_or(anyhow!("Node has no client, can't send remote signal!"))?
			.borrow()
			.get_messenger()
			.execute_remote_method(self.path.as_str(), method, data, callback)
			.map_err(|_| anyhow!("Unable to write in messenger"))
	}
}

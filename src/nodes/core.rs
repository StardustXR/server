use crate::core::client::Client;
use anyhow::{anyhow, ensure, Result};
use libstardustxr::messenger::Messenger;
use rccell::{RcCell, WeakCell};
use std::{collections::HashMap, rc::Weak, vec::Vec};

use super::spatial::Spatial;

pub type Signal<'a> = dyn Fn(&[u8]) + 'a;
pub type Method<'a> = dyn Fn(&[u8]) -> Vec<u8> + 'a;

pub type NodeRef<'a> = WeakCell<Node<'a>>;

pub enum NodeData<'a> {
	None,
	Spatial(Spatial<'a>),
}

pub struct Node<'a> {
	path: String,
	trailing_slash_pos: usize,
	pub messenger: Weak<Messenger<'a>>,
	local_signals: HashMap<String, Box<Signal<'a>>>,
	local_methods: HashMap<String, Box<Method<'a>>>,

	pub data: NodeData<'a>,
}

impl<'a> Node<'a> {
	pub fn get_name(&self) -> &str {
		&self.path[self.trailing_slash_pos + 1..]
	}
	pub fn get_path(&self) -> &str {
		self.path.as_str()
	}

	pub fn from_path(
		client: Option<&mut Client<'a>>,
		path: &str,
		data_closure: impl FnOnce(NodeRef<'a>) -> NodeData<'a>,
	) -> Result<NodeRef<'a>> {
		ensure!(path.starts_with('/'), "Invalid path {}", path);
		let mut weak_messenger = Weak::default();
		client
			.as_ref()
			.map(|c| weak_messenger = c.get_weak_messenger());
		let node = Node {
			path: path.to_string(),
			trailing_slash_pos: path
				.rfind('/')
				.ok_or_else(|| anyhow!("Invalid path {}", path))?,
			messenger: weak_messenger,
			local_signals: HashMap::new(),
			local_methods: HashMap::new(),

			data: NodeData::None,
		};
		let node_ref = RcCell::new(node);
		let weak_node = node_ref.downgrade();
		node_ref.borrow_mut().data = data_closure(weak_node.clone());
		client.map(|c| c.scenegraph.as_mut().unwrap().add_node(node_ref));
		Ok(weak_node)
	}

	pub fn send_local_signal(&self, method: &str, data: &[u8]) -> Result<()> {
		self.local_signals
			.get(method)
			.ok_or_else(|| anyhow!("Signal {} not found", method))?(data);
		Ok(())
	}
	pub fn execute_local_method(&self, method: &str, data: &[u8]) -> Result<Vec<u8>> {
		Ok(self
			.local_methods
			.get(method)
			.ok_or_else(|| anyhow!("Method {} not found", method))?(data))
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

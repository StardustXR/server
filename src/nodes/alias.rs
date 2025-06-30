use super::{Aspect, AspectIdentifier, Node};
use crate::core::{client::Client, error::Result, registry::Registry};
use std::{
	ops::Add,
	sync::{Arc, Weak},
};

#[derive(Debug, Default, Clone)]
pub struct AliasInfo {
	pub(super) server_signals: Vec<u64>,
	pub(super) server_methods: Vec<u64>,
	pub(super) client_signals: Vec<u64>,
}
impl Add for AliasInfo {
	type Output = AliasInfo;
	fn add(mut self, mut rhs: Self) -> Self::Output {
		self.server_signals.append(&mut rhs.server_signals);
		self.server_methods.append(&mut rhs.server_methods);
		self.client_signals.append(&mut rhs.client_signals);
		self
	}
}

#[derive(Debug)]
pub struct Alias {
	pub(super) node: Weak<Node>,
	pub(super) original: Weak<Node>,
	pub(super) info: AliasInfo,
}
impl Alias {
	pub fn create(
		original: &Arc<Node>,
		client: &Arc<Client>,
		info: AliasInfo,
		list: Option<&AliasList>,
	) -> Result<Arc<Node>> {
		let node = Node::generate(client, true).add_to_scenegraph()?;
		Self::add_to(&node, original, info)?;
		if let Some(list) = list {
			list.add(&node);
		}
		Ok(node)
	}
	pub fn create_with_id(
		original: &Arc<Node>,
		client: &Arc<Client>,
		new_id: u64,
		info: AliasInfo,
		list: Option<&AliasList>,
	) -> Result<Arc<Node>> {
		let node = Node::from_id(client, new_id, true).add_to_scenegraph()?;
		Self::add_to(&node, original, info)?;
		if let Some(list) = list {
			list.add(&node);
		}
		Ok(node)
	}

	fn add_to(new_node: &Arc<Node>, original: &Arc<Node>, info: AliasInfo) -> Result<()> {
		let alias = Alias {
			node: Arc::downgrade(new_node),
			original: Arc::downgrade(original),
			info,
		};
		let alias = original.aliases.add(alias);
		new_node.add_aspect_raw(alias);
		Ok(())
	}
}
impl AspectIdentifier for Alias {
	const ID: u64 = 0;
}
impl Aspect for Alias {
	fn as_any(self: Arc<Self>) -> Arc<dyn std::any::Any + Send + Sync + 'static> {
		self
	}
	fn run_signal(
		&self,
		_calling_client: Arc<Client>,
		_node: Arc<Node>,
		_signal: u64,
		_message: super::Message,
	) -> Result<(), stardust_xr::scenegraph::ScenegraphError> {
		Ok(())
	}
	fn run_method(
		&self,
		_calling_client: Arc<Client>,
		_node: Arc<Node>,
		_method: u64,
		_message: super::Message,
		_response: crate::core::scenegraph::MethodResponseSender,
	) {
	}
}

pub fn get_original(node: Arc<Node>, stop_on_disabled: bool) -> Option<Arc<Node>> {
	let Ok(alias) = node.get_aspect::<Alias>() else {
		return Some(node);
	};
	if stop_on_disabled && !node.enabled() {
		return None;
	}
	get_original(alias.original.upgrade()?, stop_on_disabled)
}
pub fn links_to(alias: Arc<Node>, original: Weak<Node>) -> bool {
	let Ok(alias) = alias.get_aspect::<Alias>() else {
		return false;
	};
	if alias.original.ptr_eq(&original) {
		return true;
	}
	let Some(original_strong) = alias.original.upgrade() else {
		return false;
	};
	links_to(original_strong, original)
}

#[derive(Debug, Default, Clone)]
pub struct AliasList(Registry<Node>);
impl AliasList {
	fn add(&self, node: &Arc<Node>) {
		self.0.add_raw(node);
	}
	pub fn get_from_original_node(&self, original: Weak<Node>) -> Option<Arc<Node>> {
		self.0
			.get_valid_contents()
			.into_iter()
			.find(move |node| links_to(node.clone(), original.clone()))
	}
	pub fn get_from_aspect<A: AspectIdentifier>(&self, aspect: &A) -> Option<Arc<Node>> {
		self.0.get_valid_contents().into_iter().find(|node| {
			let Some(node) = get_original(node.clone(), false) else {
				return false;
			};
			let Ok(aspect2) = node.get_aspect::<A>() else {
				return false;
			};
			std::ptr::eq(Arc::as_ptr(&aspect2), aspect)
		})
	}
	pub fn get_aliases(&self) -> Vec<Arc<Node>> {
		self.0.get_valid_contents()
	}
	pub fn remove_aspect<A: AspectIdentifier>(&self, aspect: &A) {
		self.0.retain(|node| {
			let Some(original) = get_original(node.clone(), false) else {
				return false;
			};
			let Ok(aspect2) = original.get_aspect::<A>() else {
				return false;
			};
			!std::ptr::eq(Arc::as_ptr(&aspect2), aspect)
		})
	}
}
impl Drop for AliasList {
	fn drop(&mut self) {
		for node in self.0.take_valid_contents() {
			node.destroy();
		}
	}
}

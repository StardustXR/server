use super::Node;
use crate::core::client::Client;
use color_eyre::eyre::{ensure, Result};
use portable_atomic::AtomicBool;
use std::sync::{Arc, Weak};

#[derive(Debug, Default, Clone)]
pub struct AliasInfo {
	pub(super) server_signals: Vec<&'static str>,
	pub(super) server_methods: Vec<&'static str>,
	pub(super) client_signals: Vec<&'static str>,
}

#[allow(dead_code)]
pub struct Alias {
	pub enabled: Arc<AtomicBool>,
	pub(super) node: Weak<Node>,
	pub original: Weak<Node>,

	pub info: AliasInfo,
}
impl Alias {
	pub fn create(
		client: &Arc<Client>,
		parent: &str,
		name: &str,
		original: &Arc<Node>,
		info: AliasInfo,
	) -> Result<Arc<Node>> {
		ensure!(
			client
				.scenegraph
				.get_node(&(parent.to_string() + "/" + name))
				.is_none(),
			"Node already exists"
		);

		let node = Node::create(client, parent, name, true).add_to_scenegraph()?;
		let alias = Alias {
			enabled: Arc::new(AtomicBool::new(true)),
			node: Arc::downgrade(&node),
			original: Arc::downgrade(original),
			info,
		};
		let alias = original.aliases.add(alias);
		let _ = node.alias.set(alias);
		Ok(node)
	}
}

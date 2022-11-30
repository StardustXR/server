use crate::core::client::Client;

use super::Node;
use std::sync::{Arc, Weak};

#[derive(Debug, Default, Clone)]
pub struct AliasInfo {
	pub(super) local_signals: Vec<&'static str>,
	pub(super) local_methods: Vec<&'static str>,
	pub(super) remote_signals: Vec<&'static str>,
}

#[allow(dead_code)]
pub struct Alias {
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
	) -> Option<Arc<Node>> {
		let node_free = client
			.scenegraph
			.get_node(&(parent.to_string() + "/" + name))
			.is_none();

		node_free.then(|| {
			let node = Node::create(client, parent, name, true).add_to_scenegraph();
			let alias = Alias {
				node: Arc::downgrade(&node),
				original: Arc::downgrade(original),
				info,
			};
			let alias = original.aliases.add(alias);
			let _ = node.alias.set(alias);
			node
		})
	}
}

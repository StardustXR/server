use super::Node;
use std::sync::{Arc, Weak};

#[allow(dead_code)]
pub struct Alias {
	pub(super) node: Weak<Node>,
	pub original: Weak<Node>,

	pub(super) local_signals: Vec<&'static str>,
	pub(super) local_methods: Vec<&'static str>,
	pub(super) remote_signals: Vec<&'static str>,
	pub(super) remote_methods: Vec<&'static str>,
}
impl Alias {
	pub fn add_to(
		node: &Arc<Node>,
		original: &Arc<Node>,
		local_signals: Vec<&'static str>,
		local_methods: Vec<&'static str>,
		remote_signals: Vec<&'static str>,
		remote_methods: Vec<&'static str>,
	) -> Arc<Alias> {
		let alias = Alias {
			node: Arc::downgrade(node),
			original: Arc::downgrade(original),
			local_signals,
			local_methods,
			remote_signals,
			remote_methods,
		};
		let alias = original.aliases.add(alias);
		let _ = node.alias.set(alias.clone());
		alias
	}
}

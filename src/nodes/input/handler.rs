use super::{
	input_handler_client, DistanceLink, InputHandlerAspect, INPUT_HANDLER_REGISTRY,
	INPUT_METHOD_REGISTRY,
};
use crate::{
	core::node_collections::LifeLinkedNodeMap,
	nodes::{fields::Field, spatial::Spatial, Aspect, Node},
};
use color_eyre::eyre::Result;
use portable_atomic::AtomicBool;
use stardust_xr::values::Datamap;
use std::sync::{Arc, Weak};
use tracing::instrument;

pub struct InputHandler {
	pub enabled: Arc<AtomicBool>,
	pub uid: String,
	pub node: Weak<Node>,
	pub spatial: Arc<Spatial>,
	pub field: Arc<Field>,
	pub(super) method_aliases: LifeLinkedNodeMap<usize>,
}
impl InputHandler {
	pub fn add_to(node: &Arc<Node>, field: &Arc<Field>) -> Result<()> {
		let handler = InputHandler {
			enabled: node.enabled.clone(),
			uid: node.uid.clone(),
			node: Arc::downgrade(node),
			spatial: node.get_aspect::<Spatial>().unwrap().clone(),
			field: field.clone(),
			method_aliases: LifeLinkedNodeMap::default(),
		};
		for method in INPUT_METHOD_REGISTRY.get_valid_contents() {
			method.make_alias(&handler);
			method.handle_new_handler(&handler);
		}
		let handler = INPUT_HANDLER_REGISTRY.add(handler);
		node.add_aspect_raw(handler);
		Ok(())
	}

	#[instrument(level = "debug", skip(self, distance_link))]
	pub(super) fn send_input(
		&self,
		order: u32,
		captured: bool,
		distance_link: &DistanceLink,
		datamap: Datamap,
	) {
		let Some(node) = self.node.upgrade() else {
			return;
		};
		let Some(method_alias) = distance_link
			.handler
			.method_aliases
			.get(&(Arc::as_ptr(&distance_link.method) as usize))
		else {
			return;
		};
		let _ = input_handler_client::input(
			&node,
			&method_alias,
			&distance_link.serialize(order, captured, datamap),
		);
	}
}
impl Aspect for InputHandler {
	const NAME: &'static str = "InputHandler";
}
impl InputHandlerAspect for InputHandler {}
impl PartialEq for InputHandler {
	fn eq(&self, other: &Self) -> bool {
		self.spatial == other.spatial
	}
}
impl Drop for InputHandler {
	fn drop(&mut self) {
		INPUT_HANDLER_REGISTRY.remove(self);
		for method in INPUT_METHOD_REGISTRY.get_valid_contents() {
			method.handle_drop_handler(self);
		}
	}
}

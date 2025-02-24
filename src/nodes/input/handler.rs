use super::{INPUT_HANDLER_REGISTRY, INPUT_METHOD_REGISTRY, InputHandlerAspect};
use crate::nodes::{Node, alias::AliasList, fields::Field, spatial::Spatial};
use color_eyre::eyre::Result;
use std::sync::Arc;

pub struct InputHandler {
	pub spatial: Arc<Spatial>,
	pub field: Arc<Field>,
	pub(super) method_aliases: AliasList,
}
impl InputHandler {
	pub fn add_to(node: &Arc<Node>, field: &Arc<Field>) -> Result<()> {
		let handler = InputHandler {
			spatial: node.get_aspect::<Spatial>().unwrap().clone(),
			field: field.clone(),
			method_aliases: AliasList::default(),
		};
		for method in INPUT_METHOD_REGISTRY.get_valid_contents() {
			method.handle_new_handler(&handler);
		}
		let handler = INPUT_HANDLER_REGISTRY.add(handler);
		node.add_aspect_raw(handler);
		Ok(())
	}
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

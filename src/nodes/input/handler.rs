use super::{INPUT_HANDLER_REGISTRY, INPUT_METHOD_REGISTRY};
use crate::nodes::{fields::Field, spatial::SpatialMut};
use color_eyre::eyre::Result;
use stardust_xr_protocol::protocol::input::InputHandlerHandler;
use std::sync::Arc;

pub struct InputHandler {
	pub spatial: Arc<SpatialMut>,
	pub field: Arc<Field>,
	// No alias storage needed - methods own the links!
}
impl InputHandler {
	pub fn add_to(node: &Arc<Node>, field: &Arc<Field>) -> Result<()> {
		let handler = InputHandler {
			spatial: node.get_aspect::<SpatialMut>().unwrap().clone(),
			field: field.clone(),
		};
		let handler_arc = INPUT_HANDLER_REGISTRY.add(handler);
		for method in INPUT_METHOD_REGISTRY.get_valid_contents() {
			method.handle_new_handler(&handler_arc);
		}
		node.add_aspect_raw(handler_arc);
		Ok(())
	}
}
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

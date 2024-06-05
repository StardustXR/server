use super::{
	input_handler_client, InputHandlerAspect, InputLink, INPUT_HANDLER_REGISTRY,
	INPUT_METHOD_REGISTRY,
};
use crate::nodes::{alias::AliasList, fields::Field, spatial::Spatial, Aspect, Node};
use color_eyre::eyre::Result;
use stardust_xr::values::Datamap;
use std::sync::Arc;
use tracing::instrument;

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
			method.make_alias(&handler);
			method.handle_new_handler(&handler);
		}
		let handler = INPUT_HANDLER_REGISTRY.add(handler);
		node.add_aspect_raw(handler);
		Ok(())
	}

	#[instrument(level = "debug", skip(self, input_link))]
	pub(super) fn send_input(
		&self,
		order: u32,
		captured: bool,
		input_link: &InputLink,
		datamap: Datamap,
	) {
		let Some(node) = self.spatial.node() else {
			return;
		};
		let Some(method_alias) = self.method_aliases.get(input_link.method.as_ref()) else {
			return;
		};
		let _ = input_handler_client::input(
			&node,
			&method_alias,
			&input_link.serialize(method_alias.id, order, captured, datamap),
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

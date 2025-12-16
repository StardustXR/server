use super::{
	INPUT_METHOD_REF_ASPECT_ALIAS_INFO, InputHandler, InputMethod, input_handler_client,
	input_method_client,
};
use crate::{
	core::{Id, error::Result},
	nodes::{Node, alias::Alias, fields::FIELD_ALIAS_INFO},
};
use std::sync::{Arc, Weak};

/// RAII type that manages the bidirectional alias relationship between a method and handler.
/// When created, sets up all aliases. When dropped, tears down all aliases and sends notifications.
pub struct InputMethodHandlerLink {
	method: Weak<InputMethod>,
	handler: Weak<InputHandler>,

	// Aliases the method's client uses to reference the handler
	method_to_handler_alias: Arc<Node>,
	method_to_handler_field_alias: Arc<Node>,

	// Alias the handler's client uses to call methods on the method
	handler_to_method_alias: Arc<Node>,
}

impl InputMethodHandlerLink {
	/// Creates all bidirectional aliases at once.
	/// This ensures we can't have half-setup state - either all aliases exist or none do.
	///
	/// Gets the Arc refs from the spatial nodes to work around borrow issues.
	pub fn create(method_node: &Arc<Node>, handler_node: &Arc<Node>) -> Result<Arc<Self>> {
		let method = method_node.get_aspect::<InputMethod>()?;
		let handler = handler_node.get_aspect::<InputHandler>()?;
		let method_client = method_node
			.get_client()
			.ok_or_else(|| color_eyre::eyre::eyre!("Method has no client"))?;
		let handler_client = handler_node
			.get_client()
			.ok_or_else(|| color_eyre::eyre::eyre!("Handler has no client"))?;
		let handler_field_node = handler
			.field
			.spatial
			.node()
			.ok_or_else(|| color_eyre::eyre::eyre!("Handler field has no node"))?;

		// Create method -> handler alias (for method's client to reference handler)
		let method_to_handler_alias = Alias::create(
			handler_node,
			&method_client,
			INPUT_METHOD_REF_ASPECT_ALIAS_INFO.clone(),
			None, // We own it, not stored in a list
		)?;

		// Create method -> handler's field alias (for method's client)
		let method_to_handler_field_alias = Alias::create(
			&handler_field_node,
			&method_client,
			FIELD_ALIAS_INFO.clone(),
			None,
		)?;

		// Create handler -> method alias (for handler's client to call methods on method)
		let handler_to_method_alias = Alias::create(
			method_node,
			&handler_client,
			INPUT_METHOD_REF_ASPECT_ALIAS_INFO.clone(),
			None,
		)?;

		// Start disabled - only enable when in active routing
		handler_to_method_alias.set_enabled(false);

		// Notify method's client about the new handler
		input_method_client::create_handler(
			method_node,
			&method_to_handler_alias,
			&method_to_handler_field_alias,
		)?;

		Ok(Arc::new(Self {
			method: Arc::downgrade(&method),
			handler: Arc::downgrade(&handler),
			method_to_handler_alias,
			method_to_handler_field_alias,
			handler_to_method_alias,
		}))
	}

	/// Get the alias the method's client uses to reference the handler
	pub fn handler_alias_for_method_client(&self) -> &Arc<Node> {
		&self.method_to_handler_alias
	}

	/// Get the alias the handler's client uses to reference the method
	pub fn method_alias_for_handler_client(&self) -> &Arc<Node> {
		&self.handler_to_method_alias
	}

	/// Get the alias ID the handler's client uses to reference the method
	pub fn method_alias_id_for_handler_client(&self) -> Id {
		self.handler_to_method_alias.id
	}

	/// Enable the method alias for the handler (when entering routing)
	pub fn enable_for_handler(&self) {
		self.handler_to_method_alias.set_enabled(true);
	}

	/// Disable the method alias for the handler (when leaving routing)
	pub fn disable_for_handler(&self) {
		self.handler_to_method_alias.set_enabled(false);
	}

	/// Check if this link is for the given handler
	pub fn is_for_handler(&self, handler: &Arc<InputHandler>) -> bool {
		self.handler
			.upgrade()
			.map(|h| Arc::ptr_eq(&h, handler))
			.unwrap_or(false)
	}

	/// Get the handler if still alive
	pub fn handler(&self) -> Option<Arc<InputHandler>> {
		self.handler.upgrade()
	}

	/// Notify handler's client that input has left
	pub fn send_input_left(&self) -> Result<()> {
		let handler = self
			.handler
			.upgrade()
			.ok_or_else(|| color_eyre::eyre::eyre!("Handler dropped"))?;
		let handler_node = handler
			.spatial
			.node()
			.ok_or_else(|| color_eyre::eyre::eyre!("Handler has no node"))?;

		input_handler_client::input_left(&handler_node, self.handler_to_method_alias.id)
	}
}

impl Drop for InputMethodHandlerLink {
	fn drop(&mut self) {
		// Automatically clean up when link is dropped
		if let Some(method) = self.method.upgrade()
			&& let Some(method_node) = method.spatial.node()
		{
			// Notify method's client that handler is destroyed
			let _ =
				input_method_client::destroy_handler(&method_node, self.method_to_handler_alias.id);
		}

		// Destroy all alias nodes
		self.method_to_handler_alias.destroy();
		self.method_to_handler_field_alias.destroy();
		self.handler_to_method_alias.destroy();
	}
}

impl PartialEq for InputMethodHandlerLink {
	fn eq(&self, other: &Self) -> bool {
		self.handler.ptr_eq(&other.handler) && self.method.ptr_eq(&other.method)
	}
}

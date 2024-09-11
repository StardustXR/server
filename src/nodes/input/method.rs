use super::{
	input_method_client, InputData, InputDataTrait, InputDataType, InputHandler, InputMethodAspect,
	InputMethodRefAspect, INPUT_HANDLER_REGISTRY, INPUT_METHOD_REF_ASPECT_ALIAS_INFO,
	INPUT_METHOD_REGISTRY,
};
use crate::{
	core::{client::Client, registry::Registry},
	nodes::{
		alias::{Alias, AliasList},
		fields::{Field, FIELD_ALIAS_INFO},
		spatial::Spatial,
		Node,
	},
};
use color_eyre::eyre::Result;
use parking_lot::Mutex;
use stardust_xr::values::Datamap;
use std::sync::{Arc, Weak};

pub struct InputMethod {
	pub spatial: Arc<Spatial>,
	pub data: Mutex<InputDataType>,
	pub datamap: Mutex<Datamap>,

	handler_aliases: AliasList,
	handler_field_aliases: AliasList,
	pub(super) handler_order: Mutex<Vec<Weak<InputHandler>>>,
	pub internal_capture_requests: Registry<InputHandler>,
	pub captures: Registry<InputHandler>,
}
impl InputMethod {
	pub fn add_to(
		node: &Arc<Node>,
		data: InputDataType,
		datamap: Datamap,
	) -> Result<Arc<InputMethod>> {
		let method = InputMethod {
			spatial: node.get_aspect::<Spatial>().unwrap().clone(),
			data: Mutex::new(data),
			datamap: Mutex::new(datamap),

			handler_aliases: AliasList::default(),
			handler_field_aliases: AliasList::default(),
			handler_order: Mutex::new(Vec::new()),
			internal_capture_requests: Registry::new(),
			captures: Registry::new(),
		};
		for handler in INPUT_HANDLER_REGISTRY.get_valid_contents() {
			method.handle_new_handler(&handler);
		}
		let method = INPUT_METHOD_REGISTRY.add(method);
		node.add_aspect_raw(method.clone());
		node.add_aspect(InputMethodRef);
		Ok(method)
	}

	pub fn distance(&self, to: &Field) -> f32 {
		self.data.lock().distance(&self.spatial, to)
	}

	pub fn set_handler_order<'a>(&self, handlers: impl Iterator<Item = &'a Arc<InputHandler>>) {
		*self.handler_order.lock() = handlers.map(Arc::downgrade).collect();
	}

	pub(super) fn make_alias(&self, handler: &InputHandler) {
		let Some(method_node) = self.spatial.node() else {
			return;
		};
		let Some(handler_node) = handler.spatial.node() else {
			return;
		};
		let Some(client) = handler_node.get_client() else {
			return;
		};
		let Ok(method_alias) = Alias::create(
			&method_node,
			&client,
			INPUT_METHOD_REF_ASPECT_ALIAS_INFO.clone(),
			Some(&handler.method_aliases),
		) else {
			return;
		};
		method_alias.set_enabled(false);
	}
	pub(super) fn handle_new_handler(&self, handler: &InputHandler) {
		self.make_alias(handler);

		let Some(method_node) = self.spatial.node() else {
			return;
		};
		let Some(method_client) = method_node.get_client() else {
			return;
		};
		let Some(handler_node) = handler.spatial.node() else {
			return;
		};
		// Receiver itself
		let Ok(handler_alias) = Alias::create(
			&handler_node,
			&method_client,
			INPUT_METHOD_REF_ASPECT_ALIAS_INFO.clone(),
			Some(&self.handler_aliases),
		) else {
			return;
		};

		let Some(handler_field_node) = handler.field.spatial.node() else {
			return;
		};
		// Handler's field
		let Ok(rx_field_alias) = Alias::create(
			&handler_field_node,
			&method_client,
			FIELD_ALIAS_INFO.clone(),
			Some(&self.handler_field_aliases),
		) else {
			return;
		};

		let _ = input_method_client::create_handler(&method_node, &handler_alias, &rx_field_alias);
	}
	pub(super) fn handle_drop_handler(&self, handler: &InputHandler) {
		let Some(tx_node) = self.spatial.node() else {
			return;
		};
		let Some(handler_alias) = self.handler_aliases.get_from_aspect(handler) else {
			return;
		};
		let _ = input_method_client::destroy_handler(&tx_node, handler_alias.id);
		self.handler_aliases.remove_aspect(handler);
		self.handler_field_aliases
			.remove_aspect(handler.field.as_ref());
	}

	pub(super) fn serialize(&self, alias_id: u64, handler: &Arc<InputHandler>) -> InputData {
		let mut input = self.data.lock().clone();
		input.transform(self, handler);

		InputData {
			id: alias_id,
			input,
			distance: self.distance(&handler.field),
			datamap: self.datamap.lock().clone(),
			order: self
				.handler_order
				.lock()
				.iter()
				.enumerate()
				.find(|(_, h)| h.ptr_eq(&Arc::downgrade(handler)))
				.unwrap()
				.0 as u32,
			captured: self.captures.get_valid_contents().contains(handler),
		}
	}
}
impl InputMethodAspect for InputMethod {
	#[doc = "Set the spatial input component of this input method. You must keep the same input data type throughout the entire thing."]
	fn set_input(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		input: InputDataType,
	) -> Result<()> {
		let input_method = node.get_aspect::<InputMethod>()?;
		*input_method.data.lock() = input;
		Ok(())
	}

	#[doc = "Set the datmap of this input method"]
	fn set_datamap(node: Arc<Node>, _calling_client: Arc<Client>, datamap: Datamap) -> Result<()> {
		let input_method = node.get_aspect::<InputMethod>()?;
		*input_method.datamap.lock() = datamap;
		Ok(())
	}

	#[doc = "Manually set the order of handlers to propagate input to, or else let the server decide."]
	fn set_handler_order(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		handlers: Vec<Arc<Node>>,
	) -> Result<()> {
		let input_method = node.get_aspect::<InputMethod>()?;
		let handlers = handlers
			.into_iter()
			.filter_map(|p| p.get_aspect::<InputHandler>().ok())
			.map(|i| Arc::downgrade(&i))
			.collect::<Vec<_>>();

		*input_method.handler_order.lock() = handlers;
		Ok(())
	}

	#[doc = "Set which handlers are captured."]
	fn set_captures(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		handlers: Vec<Arc<Node>>,
	) -> Result<()> {
		let input_method = node.get_aspect::<InputMethod>()?;
		input_method.captures.clear();
		for handler in handlers {
			let Ok(handler) = handler.get_aspect::<InputHandler>() else {
				continue;
			};
			input_method.captures.add_raw(&handler);
		}
		Ok(())
	}
}
impl Drop for InputMethod {
	fn drop(&mut self) {
		INPUT_METHOD_REGISTRY.remove(self);
	}
}

pub struct InputMethodRef;
impl InputMethodRefAspect for InputMethodRef {
	#[doc = "Have the input handler that this method reference came from capture the method for the next frame."]
	fn request_capture(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		handler: Arc<Node>,
	) -> Result<()> {
		let input_method = node.get_aspect::<InputMethod>()?;
		let input_handler = handler.get_aspect::<InputHandler>()?;

		input_method
			.internal_capture_requests
			.add_raw(&input_handler);
		Ok(())
	}
}

use super::{
	input_method_client, InputDataTrait, InputDataType, InputHandler, InputMethodAspect,
	InputMethodRefAspect, INPUT_HANDLER_REGISTRY, INPUT_METHOD_REGISTRY,
};
use crate::{
	core::{client::Client, node_collections::LifeLinkedNodeMap, registry::Registry},
	nodes::{
		alias::{Alias, AliasInfo},
		fields::{Field, FIELD_ALIAS_INFO},
		spatial::Spatial,
		Aspect, Node,
	},
};
use color_eyre::eyre::Result;
use parking_lot::Mutex;
use portable_atomic::Ordering;
use stardust_xr::values::Datamap;
use std::sync::{Arc, Weak};

pub struct InputMethod {
	pub node: Weak<Node>,
	pub uid: String,
	pub enabled: Mutex<bool>,
	pub spatial: Arc<Spatial>,
	pub data: Mutex<InputDataType>,
	pub datamap: Mutex<Datamap>,

	pub capture_requests: Registry<InputHandler>,
	pub captures: Registry<InputHandler>,
	pub(super) handler_aliases: LifeLinkedNodeMap<String>,
	pub(super) handler_order: Mutex<Vec<Weak<InputHandler>>>,
}
impl InputMethod {
	pub fn add_to(
		node: &Arc<Node>,
		data: InputDataType,
		datamap: Datamap,
	) -> Result<Arc<InputMethod>> {
		let method = InputMethod {
			node: Arc::downgrade(node),
			uid: node.uid.clone(),
			enabled: Mutex::new(true),
			spatial: node.get_aspect::<Spatial>().unwrap().clone(),
			data: Mutex::new(data),
			capture_requests: Registry::new(),
			captures: Registry::new(),
			datamap: Mutex::new(datamap),
			handler_aliases: LifeLinkedNodeMap::default(),
			handler_order: Mutex::new(Vec::new()),
		};
		for handler in INPUT_HANDLER_REGISTRY.get_valid_contents() {
			method.handle_new_handler(&handler);
			method.make_alias(&handler);
		}
		let method = INPUT_METHOD_REGISTRY.add(method);
		<InputMethod as InputMethodRefAspect>::add_node_members(node);
		<InputMethod as InputMethodAspect>::add_node_members(node);
		node.add_aspect_raw(method.clone());
		Ok(method)
	}

	pub(super) fn make_alias(&self, handler: &InputHandler) {
		let Some(method_node) = self.node.upgrade() else {
			return;
		};
		let Some(handler_node) = handler.node.upgrade() else {
			return;
		};
		let Some(client) = handler_node.get_client() else {
			return;
		};
		let Ok(method_alias) = Alias::create(
			&client,
			handler_node.get_path(),
			&self.uid,
			&method_node,
			AliasInfo {
				server_signals: vec!["capture"],
				..Default::default()
			},
		) else {
			return;
		};
		method_alias.enabled.store(false, Ordering::Relaxed);
		handler
			.method_aliases
			.add(self as *const InputMethod as usize, &method_alias);
	}

	pub fn distance(&self, to: &Field) -> f32 {
		self.data.lock().distance(&self.spatial, to)
	}

	pub fn set_handler_order<'a>(&self, handlers: impl Iterator<Item = &'a Arc<InputHandler>>) {
		*self.handler_order.lock() = handlers.map(Arc::downgrade).collect();
	}

	pub(super) fn handle_new_handler(&self, handler: &InputHandler) {
		let Some(method_node) = self.node.upgrade() else {
			return;
		};
		let Some(method_client) = method_node.get_client() else {
			return;
		};
		let Some(handler_node) = handler.node.upgrade() else {
			return;
		};
		// Receiver itself
		let Ok(handler_alias) = Alias::create(
			&method_client,
			method_node.get_path(),
			handler.uid.as_str(),
			&handler_node,
			AliasInfo {
				server_methods: vec!["get_transform"],
				..Default::default()
			},
		) else {
			return;
		};
		self.handler_aliases
			.add(handler.uid.clone(), &handler_alias);

		let Some(handler_field_node) = handler.field.spatial_ref().node.upgrade() else {
			return;
		};
		// Handler's field
		let Ok(rx_field_alias) = Alias::create(
			&method_client,
			handler_alias.get_path(),
			"field",
			&handler_field_node,
			FIELD_ALIAS_INFO.clone(),
		) else {
			return;
		};
		self.handler_aliases
			.add(handler.uid.clone() + "-field", &rx_field_alias);

		let _ = input_method_client::create_handler(
			&method_node,
			&handler.uid,
			&handler_node,
			&rx_field_alias,
		);
	}
	pub(super) fn handle_drop_handler(&self, handler: &InputHandler) {
		let uid = handler.uid.as_str();
		self.handler_aliases.remove(uid);
		self.handler_aliases.remove(&(uid.to_string() + "-field"));
		let Some(tx_node) = self.node.upgrade() else {
			return;
		};

		let _ = input_method_client::destroy_handler(&tx_node, &uid);
	}
}
impl Aspect for InputMethod {
	const NAME: &'static str = "InputMethod";
}
impl InputMethodRefAspect for InputMethod {
	#[doc = "Have the input handler that this method reference came from capture the method for the next frame."]
	fn request_capture(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		handler: Arc<Node>,
	) -> Result<()> {
		let input_method = node.get_aspect::<InputMethod>()?;
		let input_handler = handler.get_aspect::<InputHandler>()?;

		input_method.capture_requests.add_raw(&input_handler);
		Ok(())
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

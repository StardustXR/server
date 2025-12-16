use super::{
	INPUT_HANDLER_REGISTRY, INPUT_METHOD_REGISTRY, InputData, InputDataTrait, InputDataType,
	InputHandler, InputMethodAspect, InputMethodHandlerLink, InputMethodRefAspect,
	input_method_client,
};
use crate::{
	core::{
		client::Client,
		error::{Result, ServerError},
		registry::{OwnedRegistry, Registry},
	},
	nodes::{Node, fields::Field, input::input_handler_client, spatial::Spatial},
};
use color_eyre::eyre::eyre;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use stardust_xr_wire::values::Datamap;
use std::sync::{Arc, Weak};

pub struct InputMethod {
	pub spatial: Arc<Spatial>,
	data: Mutex<InputDataType>,
	datamap: Mutex<Datamap>,

	// All bidirectional aliases managed by links
	handler_links: OwnedRegistry<InputMethodHandlerLink>,
	pub(super) handler_order: Mutex<Vec<Weak<InputHandler>>>,
	pub capture_attempts: Registry<InputHandler>,
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

			handler_links: OwnedRegistry::new(),
			handler_order: Mutex::new(Vec::new()),
			capture_attempts: Registry::new(),
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

	// Helper to find a link for a given handler
	fn find_link(&self, handler: &Arc<InputHandler>) -> Option<Arc<InputMethodHandlerLink>> {
		let handler_ptr = Arc::as_ptr(handler) as usize;
		self.handler_links.get_vec().into_iter().find(|link| {
			link.handler()
				.map(|h| Arc::as_ptr(&h) as usize == handler_ptr)
				.unwrap_or(false)
		})
	}

	pub(super) fn handle_new_handler(&self, handler: &Arc<InputHandler>) {
		// Create link - this sets up all bidirectional aliases and sends notifications
		let Some(method_node) = self.spatial.node() else {
			return;
		};
		let Some(handler_node) = handler.spatial.node() else {
			return;
		};
		if let Ok(link) = InputMethodHandlerLink::create(&method_node, &handler_node) {
			self.handler_links.add_raw(link);
		}
	}
	pub(super) fn handle_drop_handler(&self, handler: &InputHandler) {
		// Find and remove the link for this handler - Drop impl will handle cleanup and notifications
		let handler_ptr = handler as *const InputHandler as usize;
		for link in self.handler_links.get_vec() {
			if link
				.handler()
				.map(|h| Arc::as_ptr(&h) as usize == handler_ptr)
				.unwrap_or(false)
			{
				self.handler_links.remove(&*link);
				break;
			}
		}

		// Also clean up captures
		self.capture_attempts.remove(handler);
		self.captures.remove(handler);
	}

	#[tracing::instrument(level = "trace", skip_all)]
	fn update_input(&self) {
		let input = self.data.lock().clone();
		let datamap = self.datamap.lock().clone();
		let mut data = InputData {
			id: 0.into(),
			input: input.clone(),
			distance: 0.0,
			datamap,
			order: 0,
			captured: false,
		};
		let handler_order = self.handler_order.lock();
		for (index, handler_weak) in handler_order.iter().enumerate() {
			let Some(handler) = handler_weak.upgrade() else {
				continue;
			};
			let Some(handler_node) = handler.spatial.node() else {
				continue;
			};

			// Find the link for this handler
			let Some(link) = self.find_link(&handler) else {
				continue;
			};

			// Use the link's method alias that the handler sees
			data.id = link.method_alias_id_for_handler_client();
			data.input = input.clone();
			data.input.transform(self, &handler);
			data.distance = self.distance(&handler.field);
			data.order = index as u32;
			data.captured = self.captures.contains(&handler);
			let _ = input_handler_client::input_updated(&handler_node, &data);
		}
	}

	pub fn update_state(&self, input: InputDataType, datamap: Datamap) {
		*self.data.lock() = input;
		*self.datamap.lock() = datamap;
		self.update_input();
	}

	pub fn set_handler_order(&self, handlers: Vec<Arc<InputHandler>>) {
		let mut handler_order_lock = self.handler_order.lock();

		// Build hashmap of old order
		let old_handler_order = FxHashMap::from_iter(
			handler_order_lock
				.iter()
				.filter_map(Weak::upgrade)
				.filter_map(|handler| {
					Some((Arc::as_ptr(&handler), (handler.spatial.node()?, handler)))
				}),
		);

		// Update the order
		*handler_order_lock = handlers.iter().map(Arc::downgrade).collect();

		// Build hashmap of new order
		let handler_order_hashset = FxHashMap::from_iter(
			handlers
				.into_iter()
				.filter_map(|handler| Some((handler.spatial.node()?, handler)))
				.enumerate()
				.map(|(i, (handler_node, handler))| {
					(Arc::as_ptr(&handler), (i, handler_node, handler))
				}),
		);

		// Remove handlers that are no longer in the new order
		for (ptr, (_old_handler_node, old_handler)) in &old_handler_order {
			if handler_order_hashset.contains_key(ptr) {
				continue; // Still in order, keep it
			}

			// Find link and send input_left before disabling
			if let Some(link) = self.find_link(old_handler) {
				let _ = link.send_input_left();
				link.disable_for_handler();
			}

			self.capture_attempts.remove(old_handler);
			self.captures.remove(old_handler);
		}

		let input = self.data.lock().clone();
		let mut data = InputData {
			id: 0.into(),
			input: input.clone(),
			distance: 0.0,
			datamap: self.datamap.lock().clone(),
			order: 0,
			captured: false,
		};

		// Add/update handlers in the new order
		for (ptr, (i, handler_node, handler)) in handler_order_hashset {
			data.input = input.clone();
			data.input.transform(self, &handler);
			data.distance = self.distance(&handler.field);
			data.order = i as u32;
			data.captured = self.captures.contains(&handler);

			// Find or create link
			let link = self.find_link(&handler);

			if let Some(link) = link {
				// Link exists
				if old_handler_order.contains_key(&ptr) {
					// Handler was in old order, just update
					data.id = link.method_alias_id_for_handler_client();
					let _ = input_handler_client::input_updated(&handler_node, &data);
				} else {
					// Handler is new to order, send input_sent and enable
					data.id = link.method_alias_id_for_handler_client();
					link.enable_for_handler();
					let _ = input_handler_client::input_sent(
						&handler_node,
						link.method_alias_for_handler_client(),
						&data,
					);
				}
			} else {
				// No link exists, create it (handler just appeared)
				let Some(method_node) = self.spatial.node() else {
					continue;
				};
				if let Ok(new_link) = InputMethodHandlerLink::create(&method_node, &handler_node) {
					data.id = new_link.method_alias_id_for_handler_client();
					new_link.enable_for_handler();
					let _ = input_handler_client::input_sent(
						&handler_node,
						new_link.method_alias_for_handler_client(),
						&data,
					);
					self.handler_links.add_raw(new_link);
				}
			}
		}
	}

	pub fn set_captures(&self, handlers: Vec<Arc<InputHandler>>) {
		self.captures.clear();
		for handler in handlers {
			self.captures.add_raw(&handler);
		}
		self.update_input();
	}

	pub fn data(
		&self,
	) -> parking_lot::lock_api::MutexGuard<'_, parking_lot::RawMutex, InputDataType> {
		self.data.lock()
	}
}
impl InputMethodAspect for InputMethod {
	#[doc = "Set the input data of this input method. You must keep the same input data type throughout the entire thing."]
	fn update_state(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		input: InputDataType,
		datamap: Datamap,
	) -> Result<()> {
		let method = node.get_aspect::<InputMethod>()?;
		method.update_state(input, datamap);
		Ok(())
	}

	#[doc = "Manually set the order of handlers to propagate input to, or else let the server decide."]
	fn set_handler_order(
		method_node: Arc<Node>,
		_calling_client: Arc<Client>,
		handlers: Vec<Arc<Node>>,
	) -> Result<()> {
		let method = method_node.get_aspect::<InputMethod>()?;
		let handlers: Vec<Arc<InputHandler>> = handlers
			.into_iter()
			.filter_map(|h| h.get_aspect::<InputHandler>().ok())
			.collect();
		method.set_handler_order(handlers);
		Ok(())
	}

	#[doc = "Set which handlers are captured."]
	fn set_captures(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		captures: Vec<Arc<Node>>,
	) -> Result<()> {
		let method = node.get_aspect::<InputMethod>()?;
		let handlers: Vec<Arc<InputHandler>> = captures
			.into_iter()
			.filter_map(|h| h.get_aspect::<InputHandler>().ok())
			.collect();
		method.set_captures(handlers);
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
	#[doc = "Try to capture the input method with the given handler. When the handler does not get input from the method, it will be released."]
	fn try_capture(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		handler: Arc<Node>,
	) -> Result<()> {
		let input_method = node.get_aspect::<InputMethod>()?;
		let input_handler = handler.get_aspect::<InputHandler>()?;

		input_method.capture_attempts.add_raw(&input_handler);

		// Find the link to get the handler alias for the method's client
		let Some(link) = input_method.find_link(&input_handler) else {
			return Err(ServerError::Report(eyre!(
				"Internal: Couldn't find handler link"
			)));
		};

		input_method_client::request_capture_handler(
			&node,
			link.handler_alias_for_method_client().id,
		)
	}

	#[doc = "If captured by this handler, release it (e.g. the object is let go of after grabbing)."]
	fn release(node: Arc<Node>, _calling_client: Arc<Client>, handler: Arc<Node>) -> Result<()> {
		let input_method = node.get_aspect::<InputMethod>()?;
		let input_handler = handler.get_aspect::<InputHandler>()?;

		input_method.capture_attempts.remove(&input_handler);

		// Find the link to get the handler alias for the method's client
		let Some(link) = input_method.find_link(&input_handler) else {
			return Err(ServerError::Report(eyre!(
				"Internal: Couldn't find handler link"
			)));
		};

		input_method_client::release_handler(&node, link.handler_alias_for_method_client().id)
	}
}

pub mod alias;
pub mod audio;
pub mod data;
pub mod drawable;
pub mod fields;
pub mod input;
pub mod items;
pub mod root;
pub mod spatial;

use self::alias::Alias;
use crate::core::client::Client;
use crate::core::registry::Registry;
use crate::core::scenegraph::MethodResponseSender;
use color_eyre::eyre::{eyre, Result};
use parking_lot::Mutex;
use portable_atomic::{AtomicBool, Ordering};
use rustc_hash::FxHashMap;
use serde::{de::DeserializeOwned, Serialize};
use stardust_xr::messenger::MessageSenderHandle;
use stardust_xr::scenegraph::ScenegraphError;
use stardust_xr::schemas::flex::{deserialize, serialize};
use std::any::{Any, TypeId};
use std::fmt::Debug;
use std::os::fd::OwnedFd;
use std::sync::{Arc, Weak};
use std::vec::Vec;

#[derive(Default)]
pub struct Message {
	pub data: Vec<u8>,
	pub fds: Vec<OwnedFd>,
}
impl From<Vec<u8>> for Message {
	fn from(data: Vec<u8>) -> Self {
		Message {
			data,
			fds: Vec::new(),
		}
	}
}
impl AsRef<[u8]> for Message {
	fn as_ref(&self) -> &[u8] {
		&self.data
	}
}

pub type Signal = fn(Arc<Node>, Arc<Client>, Message) -> Result<()>;
pub type Method = fn(Arc<Node>, Arc<Client>, Message, MethodResponseSender);

stardust_xr_server_codegen::codegen_node_protocol!();

pub struct OwnedNode(pub Arc<Node>);
impl Drop for OwnedNode {
	fn drop(&mut self) {
		self.0.destroy();
	}
}

pub struct Node {
	enabled: AtomicBool,
	id: u64,
	client: Weak<Client>,
	message_sender_handle: Option<MessageSenderHandle>,

	local_signals: Mutex<FxHashMap<u64, Signal>>,
	local_methods: Mutex<FxHashMap<u64, Method>>,
	aliases: Registry<Alias>,
	aspects: Aspects,
	destroyable: bool,
}
impl Node {
	pub fn get_client(&self) -> Option<Arc<Client>> {
		self.client.upgrade()
	}
	pub fn get_id(&self) -> u64 {
		self.id
	}

	pub fn generate(client: &Arc<Client>, destroyable: bool) -> Self {
		Self::from_id(client, client.generate_id(), destroyable)
	}
	pub fn from_id(client: &Arc<Client>, id: u64, destroyable: bool) -> Self {
		let node = Node {
			enabled: AtomicBool::new(true),
			client: Arc::downgrade(client),
			message_sender_handle: client.message_sender_handle.clone(),
			id,
			local_signals: Default::default(),
			local_methods: Default::default(),
			aliases: Default::default(),
			aspects: Default::default(),
			destroyable,
		};
		<Node as OwnedAspect>::add_node_members(&node);
		node
	}
	pub fn add_to_scenegraph(self) -> Result<Arc<Node>> {
		Ok(self
			.get_client()
			.ok_or_else(|| eyre!("Internal: Unable to get client"))?
			.scenegraph
			.add_node(self))
	}
	pub fn add_to_scenegraph_owned(self) -> Result<OwnedNode> {
		Ok(OwnedNode(
			self.get_client()
				.ok_or_else(|| eyre!("Internal: Unable to get client"))?
				.scenegraph
				.add_node(self),
		))
	}
	pub fn enabled(&self) -> bool {
		self.enabled.load(Ordering::Relaxed)
	}
	pub fn set_enabled(&self, enabled: bool) {
		self.enabled.store(enabled, Ordering::Relaxed)
	}
	pub fn destroy(&self) {
		if let Some(client) = self.get_client() {
			client.scenegraph.remove_node(self.get_id());
		}
	}

	// very much up for debate if we should allow this, as you can match objects using this
	// pub fn get_client_pid_flex(
	// 	node: Arc<Node>,
	// 	_calling_client: Arc<Client>,
	// 	_message: Message,
	// ) -> Result<Message> {
	// 	let client = node
	// 		.client
	// 		.upgrade()
	// 		.ok_or_else(|| eyre!("Could not get client for node?"))?;
	// 	let pid = client.pid.ok_or_else(|| eyre!("Client PID is unknown"))?;
	// 	Ok(serialize(pid)?.into())
	// }

	pub fn add_local_signal(&self, id: u64, signal: Signal) {
		self.local_signals.lock().insert(id, signal);
	}
	pub fn add_local_method(&self, id: u64, method: Method) {
		self.local_methods.lock().insert(id, method);
	}

	pub fn add_aspect<A: Aspect>(&self, aspect: A) -> Arc<A> {
		self.aspects.add(aspect)
	}
	pub fn add_aspect_raw<A: Aspect>(&self, aspect: Arc<A>) {
		self.aspects.add_raw(aspect)
	}
	pub fn get_aspect<A: Aspect>(&self) -> Result<Arc<A>> {
		self.aspects.get()
	}

	pub fn send_local_signal(
		self: Arc<Self>,
		calling_client: Arc<Client>,
		method: u64,
		message: Message,
	) -> Result<(), ScenegraphError> {
		if let Ok(alias) = self.get_aspect::<Alias>() {
			if !alias.info.server_signals.iter().any(|e| *e == method) {
				return Err(ScenegraphError::SignalNotFound);
			}
			alias
				.original
				.upgrade()
				.ok_or(ScenegraphError::BrokenAlias)?
				.send_local_signal(calling_client, method, message)
		} else {
			let signal = self
				.local_signals
				.lock()
				.get(&method)
				.cloned()
				.ok_or(ScenegraphError::SignalNotFound)?;
			signal(self, calling_client, message).map_err(|error| ScenegraphError::SignalError {
				error: error.to_string(),
			})
		}
	}
	pub fn execute_local_method(
		self: Arc<Self>,
		calling_client: Arc<Client>,
		method: u64,
		message: Message,
		response: MethodResponseSender,
	) {
		if let Ok(alias) = self.get_aspect::<Alias>() {
			if !alias.info.server_methods.iter().any(|e| *e == method) {
				response.send(Err(ScenegraphError::MethodNotFound));
				return;
			}
			let Some(alias) = alias.original.upgrade() else {
				response.send(Err(ScenegraphError::BrokenAlias));
				return;
			};
			alias.execute_local_method(
				calling_client,
				method,
				Message {
					data: message.data.clone(),
					fds: Vec::new(),
				},
				response,
			)
		} else {
			let Some(method) = self.local_methods.lock().get(&method).cloned() else {
				response.send(Err(ScenegraphError::MethodNotFound));
				return;
			};
			method(self, calling_client, message, response);
		}
	}
	pub fn send_remote_signal(&self, method: u64, message: impl Into<Message>) -> Result<()> {
		let message = message.into();
		self.aliases
			.get_valid_contents()
			.iter()
			.filter(|alias| alias.info.client_signals.iter().any(|e| e == &method))
			.filter_map(|alias| alias.node.upgrade())
			.for_each(|node| {
				// Beware! file descriptors will not be sent to aliases!!!
				let _ = node.send_remote_signal(
					method,
					Message {
						data: message.data.clone(),
						fds: Vec::new(),
					},
				);
			});
		if let Some(handle) = self.message_sender_handle.as_ref() {
			handle.signal(self.id, method, &message.data, message.fds)?;
		}
		Ok(())
	}
	pub async fn execute_remote_method_typed<S: Serialize, D: DeserializeOwned>(
		&self,
		method: u64,
		input: S,
		fds: Vec<OwnedFd>,
	) -> Result<(D, Vec<OwnedFd>)> {
		let message_sender_handle = self
			.message_sender_handle
			.as_ref()
			.ok_or(eyre!("Messenger does not exist for this node"))?;

		let serialized = serialize(input)?;
		let result = message_sender_handle
			.method(self.id, method, &serialized, fds)?
			.await
			.map_err(|e| eyre!(e))?;

		let (message, fds) = result.into_components();
		let deserialized: D = deserialize(&message)?;
		Ok((deserialized, fds))
	}
}
impl Debug for Node {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Node")
			.field("id", &self.id)
			.field("local_signals", &self.local_signals.lock().keys())
			.field("local_methods", &self.local_methods.lock().keys())
			.field("destroyable", &self.destroyable)
			.finish()
	}
}
impl OwnedAspect for Node {
	fn set_enabled(node: Arc<Node>, _calling_client: Arc<Client>, enabled: bool) -> Result<()> {
		node.set_enabled(enabled);
		Ok(())
	}

	fn destroy(node: Arc<Node>, _calling_client: Arc<Client>) -> Result<()> {
		if node.destroyable {
			node.destroy();
		}
		Ok(())
	}
}
impl Drop for Node {
	fn drop(&mut self) {
		// Debug breakpoint
	}
}

pub trait Aspect: Any + Send + Sync + 'static {
	const NAME: &'static str;
}

#[derive(Default)]
struct Aspects(Mutex<FxHashMap<TypeId, Arc<dyn Any + Send + Sync + 'static>>>);

impl Aspects {
	fn add<A: Aspect>(&self, t: A) -> Arc<A> {
		let aspect = Arc::new(t);
		self.add_raw(aspect.clone());
		aspect
	}
	fn add_raw<A: Aspect>(&self, aspect: Arc<A>) {
		self.0.lock().insert(Self::type_key::<A>(), aspect);
	}
	fn get<A: Aspect>(&self) -> Result<Arc<A>> {
		self.0
			.lock()
			.get(&Self::type_key::<A>())
			.and_then(|a| Arc::downcast(a.clone()).ok())
			.ok_or(eyre!("Couldn't get aspect {}", A::NAME.to_lowercase()))
	}

	fn type_key<A: 'static>() -> TypeId {
		TypeId::of::<A>()
	}
}
impl Drop for Aspects {
	fn drop(&mut self) {
		self.0.lock().clear()
	}
}

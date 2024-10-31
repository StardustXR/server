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
use spatial::Spatial;
use stardust_xr::messenger::MessageSenderHandle;
use stardust_xr::scenegraph::ScenegraphError;
use stardust_xr::schemas::flex::{deserialize, serialize};
use std::any::Any;
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

stardust_xr_server_codegen::codegen_node_protocol!();

pub struct Owned;
impl Aspect for Owned {
	impl_aspect_for_owned_aspect! {}
}
impl OwnedAspect for Owned {
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
			aliases: Default::default(),
			aspects: Default::default(),
			destroyable,
		};
		node.aspects.add(Owned);
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
			&& if let Ok(spatial) = self.get_aspect::<Spatial>() {
				spatial
					.global_transform()
					.to_scale_rotation_translation()
					.0
					.length_squared()
					> 0.0
			} else {
				true
			}
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
		aspect_id: u64,
		method: u64,
		message: Message,
	) -> Result<(), ScenegraphError> {
		if let Ok(alias) = self.get_aspect::<Alias>() {
			if !alias.info.server_signals.iter().any(|e| *e == method) {
				return Err(ScenegraphError::MemberNotFound);
			}
			alias
				.original
				.upgrade()
				.ok_or(ScenegraphError::BrokenAlias)?
				.send_local_signal(calling_client, aspect_id, method, message)
		} else {
			let aspect = self
				.aspects
				.0
				.lock()
				.get(&aspect_id)
				.ok_or(ScenegraphError::AspectNotFound)?
				.clone();
			aspect
				.run_signal(calling_client, self.clone(), method, message)
				.map_err(|error| ScenegraphError::MemberError {
					error: error.to_string(),
				})
		}
	}
	pub fn execute_local_method(
		self: Arc<Self>,
		calling_client: Arc<Client>,
		aspect_id: u64,
		method: u64,
		message: Message,
		response: MethodResponseSender,
	) {
		if let Ok(alias) = self.get_aspect::<Alias>() {
			if !alias.info.server_methods.iter().any(|e| *e == method) {
				response.send(Err(ScenegraphError::MemberNotFound));
				return;
			}
			let Some(alias) = alias.original.upgrade() else {
				response.send(Err(ScenegraphError::BrokenAlias));
				return;
			};
			alias.execute_local_method(
				calling_client,
				aspect_id,
				method,
				Message {
					data: message.data.clone(),
					fds: Vec::new(),
				},
				response,
			)
		} else {
			let Some(aspect) = self.aspects.0.lock().get(&aspect_id).cloned() else {
				response.send(Err(ScenegraphError::AspectNotFound));
				return;
			};
			aspect.run_method(calling_client, self.clone(), method, message, response);
		}
	}
	pub fn send_remote_signal(
		&self,
		aspect_id: u64,
		method: u64,
		message: impl Into<Message>,
	) -> Result<()> {
		let message = message.into();
		self.aliases
			.get_valid_contents()
			.iter()
			.filter(|alias| alias.info.client_signals.iter().any(|e| e == &method))
			.filter_map(|alias| alias.node.upgrade())
			.for_each(|node| {
				// Beware! file descriptors will not be sent to aliases!!!
				let _ = node.send_remote_signal(
					aspect_id,
					method,
					Message {
						data: message.data.clone(),
						fds: Vec::new(),
					},
				);
			});
		if let Some(handle) = self.message_sender_handle.as_ref() {
			handle.signal(self.id, aspect_id, method, &message.data, message.fds)?;
		}
		Ok(())
	}
	pub async fn execute_remote_method_typed<S: Serialize, D: DeserializeOwned>(
		&self,
		aspect_id: u64,
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
			.method(self.id, aspect_id, method, &serialized, fds)?
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
			.field("destroyable", &self.destroyable)
			.finish()
	}
}
impl Drop for Node {
	fn drop(&mut self) {
		// Debug breakpoint
	}
}

pub trait Aspect: Any + Send + Sync + 'static {
	fn id(&self) -> u64;
	fn as_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync + 'static>;
	fn run_signal(
		&self,
		calling_client: Arc<Client>,
		node: Arc<Node>,
		signal: u64,
		message: Message,
	) -> Result<(), stardust_xr::scenegraph::ScenegraphError>;
	fn run_method(
		&self,
		calling_client: Arc<Client>,
		node: Arc<Node>,
		method: u64,
		message: Message,
		response: MethodResponseSender,
	);
}

#[derive(Default)]
struct Aspects(Mutex<FxHashMap<u64, Arc<dyn Aspect>>>);
impl Aspects {
	fn add<A: Aspect>(&self, t: A) -> Arc<A> {
		let aspect = Arc::new(t);
		self.add_raw(aspect.clone());
		aspect
	}
	fn add_raw<A: Aspect>(&self, aspect: Arc<A>) {
		let id = aspect.id();
		self.0.lock().insert(id, aspect);
	}
	fn get<A: Aspect>(&self) -> Result<Arc<A>> {
		self.0
			.lock()
			.values()
			.find_map(|a| Arc::downcast(a.clone().as_any()).ok())
			.ok_or(eyre!("Couldn't get aspect"))
	}
}
impl Drop for Aspects {
	fn drop(&mut self) {
		self.0.lock().clear()
	}
}

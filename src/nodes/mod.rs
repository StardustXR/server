pub mod alias;
pub mod data;
pub mod drawable;
pub mod fields;
pub mod hmd;
pub mod input;
pub mod items;
pub mod root;
pub mod spatial;
pub mod startup;
pub mod sound;

use color_eyre::eyre::{eyre, Result};
use nanoid::nanoid;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use stardust_xr::messenger::MessageSenderHandle;
use stardust_xr::scenegraph::ScenegraphError;
use std::fmt::Debug;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::vec::Vec;
use tracing::{debug_span, instrument};

use core::hash::BuildHasherDefault;
use dashmap::DashMap;
use rustc_hash::FxHasher;

use crate::core::client::Client;
use crate::core::registry::Registry;

use self::alias::Alias;
use self::data::{PulseReceiver, PulseSender};

use self::drawable::lines::Lines;
use self::drawable::model::Model;
use self::drawable::text::Text;
use self::fields::Field;
use self::input::{InputHandler, InputMethod};
use self::items::{Item, ItemAcceptor, ItemUI};
use self::sound::Sound;
use self::spatial::zone::Zone;
use self::spatial::Spatial;
use self::startup::StartupSettings;

pub type Signal = fn(&Node, Arc<Client>, &[u8]) -> Result<()>;
pub type Method = fn(&Node, Arc<Client>, &[u8]) -> Result<Vec<u8>>;

pub struct Node {
	pub(super) uid: String,
	path: String,
	client: Weak<Client>,
	message_sender_handle: Option<MessageSenderHandle>,
	// trailing_slash_pos: usize,
	local_signals: DashMap<String, Signal, BuildHasherDefault<FxHasher>>,
	local_methods: DashMap<String, Method, BuildHasherDefault<FxHasher>>,
	destroyable: AtomicBool,

	pub alias: OnceCell<Arc<Alias>>,
	aliases: Registry<Alias>,

	pub spatial: OnceCell<Arc<Spatial>>,
	pub field: OnceCell<Arc<Field>>,
	pub zone: OnceCell<Arc<Zone>>,

	// Data
	pub pulse_sender: OnceCell<Arc<PulseSender>>,
	pub pulse_receiver: OnceCell<Arc<PulseReceiver>>,

	// Drawable
	pub lines: OnceCell<Arc<Lines>>,
	pub model: OnceCell<Arc<Model>>,
	pub text: OnceCell<Arc<Text>>,

	// Input
	pub input_method: OnceCell<Arc<InputMethod>>,
	pub input_handler: OnceCell<Arc<InputHandler>>,

	// Item
	pub item: OnceCell<Arc<Item>>,
	pub item_acceptor: OnceCell<Arc<ItemAcceptor>>,
	pub item_ui: OnceCell<Arc<ItemUI>>,

	// Sound
	pub sound: OnceCell<Arc<Sound>>,

	// Startup
	pub startup_settings: OnceCell<Mutex<StartupSettings>>,
}

impl Node {
	pub fn get_client(&self) -> Option<Arc<Client>> {
		self.client.upgrade()
	}
	// pub fn get_name(&self) -> &str {
	// 	&self.path[self.trailing_slash_pos + 1..]
	// }
	pub fn get_path(&self) -> &str {
		self.path.as_str()
	}
	pub fn is_destroyable(&self) -> bool {
		self.destroyable.load(Ordering::Relaxed)
	}

	pub fn create(client: &Arc<Client>, parent: &str, name: &str, destroyable: bool) -> Self {
		let mut path = parent.to_string();
		path.push('/');
		path.push_str(name);
		let node = Node {
			uid: nanoid!(),
			client: Arc::downgrade(client),
			message_sender_handle: client.message_sender_handle.clone(),
			path,
			// trailing_slash_pos: parent.len(),
			local_signals: Default::default(),
			local_methods: Default::default(),
			destroyable: AtomicBool::from(destroyable),

			alias: OnceCell::new(),
			aliases: Registry::new(),

			spatial: OnceCell::new(),
			field: OnceCell::new(),
			zone: OnceCell::new(),
			pulse_sender: OnceCell::new(),
			pulse_receiver: OnceCell::new(),
			lines: OnceCell::new(),
			model: OnceCell::new(),
			text: OnceCell::new(),
			input_method: OnceCell::new(),
			input_handler: OnceCell::new(),
			item: OnceCell::new(),
			item_acceptor: OnceCell::new(),
			item_ui: OnceCell::new(),
			sound: OnceCell::new(),
			startup_settings: OnceCell::new(),
		};
		node.add_local_signal("destroy", Node::destroy_flex);
		node
	}
	pub fn add_to_scenegraph(self) -> Result<Arc<Node>> {
		Ok(self
			.get_client()
			.ok_or_else(|| eyre!("Internal: Unable to get client"))?
			.scenegraph
			.add_node(self))
	}
	pub fn destroy(&self) {
		if let Some(client) = self.get_client() {
			client.scenegraph.remove_node(self.get_path());
		}
	}

	pub fn destroy_flex(node: &Node, _calling_client: Arc<Client>, _data: &[u8]) -> Result<()> {
		if node.is_destroyable() {
			node.destroy();
		}
		Ok(())
	}

	pub fn add_local_signal(&self, name: &str, signal: Signal) {
		self.local_signals.insert(name.to_string(), signal);
	}
	pub fn add_local_method(&self, name: &str, method: Method) {
		self.local_methods.insert(name.to_string(), method);
	}

	pub fn get_aspect<F, T>(
		&self,
		node_name: &'static str,
		aspect_type: &'static str,
		aspect_fn: F,
	) -> Result<&T>
	where
		F: FnOnce(&Node) -> &OnceCell<T>,
	{
		aspect_fn(self)
			.get()
			.ok_or_else(|| eyre!("{} is not a {} node", node_name, aspect_type))
	}

	pub fn send_local_signal(
		&self,
		calling_client: Arc<Client>,
		method: &str,
		data: &[u8],
	) -> Result<(), ScenegraphError> {
		if let Some(alias) = self.alias.get() {
			if !alias.info.local_signals.iter().any(|e| e == &method) {
				return Err(ScenegraphError::SignalNotFound);
			}
			alias
				.original
				.upgrade()
				.ok_or(ScenegraphError::BrokenAlias)?
				.send_local_signal(calling_client, method, data)
		} else {
			let signal = self
				.local_signals
				.get(method)
				.ok_or(ScenegraphError::SignalNotFound)?;
			signal(self, calling_client, data).map_err(|error| ScenegraphError::SignalError {
				error: error.to_string(),
			})
		}
	}
	pub fn execute_local_method(
		&self,
		calling_client: Arc<Client>,
		method: &str,
		data: &[u8],
	) -> Result<Vec<u8>, ScenegraphError> {
		if let Some(alias) = self.alias.get() {
			if !alias.info.local_methods.iter().any(|e| e == &method) {
				return Err(ScenegraphError::MethodNotFound);
			}
			alias
				.original
				.upgrade()
				.ok_or(ScenegraphError::BrokenAlias)?
				.execute_local_method(calling_client, method, data)
		} else {
			let method = self
				.local_methods
				.get(method)
				.ok_or(ScenegraphError::MethodNotFound)?;

			debug_span!("Handle method").in_scope(|| {
				method(self, calling_client, data).map_err(|error| ScenegraphError::MethodError {
					error: error.to_string(),
				})
			})
		}
	}
	#[instrument(level = "debug", skip_all)]
	pub fn send_remote_signal(&self, method: &str, data: &[u8]) -> Result<()> {
		self.aliases
			.get_valid_contents()
			.iter()
			.filter(|alias| alias.info.remote_signals.iter().any(|e| e == &method))
			.filter_map(|alias| alias.node.upgrade())
			.for_each(|node| {
				let _ = node.send_remote_signal(method, data);
			});
		let path = self.path.clone();
		let method = method.to_string();
		let data = data.to_vec();
		self.message_sender_handle
			.as_ref()
			.map(|handle| handle.signal(path.as_str(), method.as_str(), data.as_slice()))
			.transpose()?;
		Ok(())
	}
	#[instrument(level = "debug", skip_all)]
	pub fn execute_remote_method(
		&self,
		method: &str,
		data: Vec<u8>,
	) -> Result<impl Future<Output = Result<Vec<u8>>>> {
		let message_sender_handle = self
			.message_sender_handle
			.as_ref()
			.ok_or(eyre!("Messenger does not exist for this node"))?;

		let future = message_sender_handle.method(self.path.as_str(), method, &data)?;

		Ok(async { future.await.map_err(|e| eyre!(e)) })
	}
}
impl Debug for Node {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Node")
			.field("uid", &self.uid)
			.field("path", &self.path)
			.finish()
	}
}

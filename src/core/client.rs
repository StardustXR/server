use super::{eventloop::EventLoop, scenegraph::Scenegraph};
use crate::{
	core::registry::Registry,
	nodes::{
		data, drawable, fields, hmd, input, items,
		root::Root,
		spatial,
		startup::{self, StartupSettings, DESKTOP_STARTUP_IDS},
		Node,
	},
};
use color_eyre::{
	eyre::{eyre, Result},
	Report,
};
use lazy_static::lazy_static;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use stardust_xr::messenger::{self, MessageSenderHandle};
use std::{
	fs,
	iter::FromIterator,
	path::PathBuf,
	sync::{Arc, Weak},
};
use tokio::{net::UnixStream, sync::Notify, task::JoinHandle};
use tracing::{info, warn};

lazy_static! {
	pub static ref CLIENTS: Registry<Client> = Registry::new();
	pub static ref INTERNAL_CLIENT: Arc<Client> = CLIENTS.add(Client {
		event_loop: Weak::new(),
		index: 0,
		pid: None,
		env: None,
		exe: None,

		stop_notifier: Default::default(),
		join_handle: OnceCell::new(),

		message_sender_handle: None,
		scenegraph: Default::default(),
		root: OnceCell::new(),
		base_resource_prefixes: Default::default(),
	});
}

pub fn get_env(pid: i32) -> Result<FxHashMap<String, String>, std::io::Error> {
	let env = fs::read_to_string(format!("/proc/{pid}/environ"))?;
	Ok(FxHashMap::from_iter(
		env.split('\0')
			.filter_map(|var| var.split_once('='))
			.map(|(k, v)| (k.to_string(), v.to_string())),
	))
}
pub fn startup_settings(env: &FxHashMap<String, String>) -> Option<StartupSettings> {
	DESKTOP_STARTUP_IDS
		.lock()
		.get(env.get("STARDUST_STARTUP_TOKEN")?)
		.cloned()
}

pub struct Client {
	event_loop: Weak<EventLoop>,
	index: usize,
	pid: Option<i32>,
	env: Option<FxHashMap<String, String>>,
	exe: Option<PathBuf>,
	stop_notifier: Arc<Notify>,
	join_handle: OnceCell<JoinHandle<Result<()>>>,

	pub message_sender_handle: Option<MessageSenderHandle>,
	pub scenegraph: Arc<Scenegraph>,
	pub root: OnceCell<Arc<Root>>,
	pub base_resource_prefixes: Mutex<Vec<PathBuf>>,
}
impl Client {
	pub fn from_connection(
		index: usize,
		event_loop: &Arc<EventLoop>,
		connection: UnixStream,
	) -> Arc<Self> {
		let pid = connection.peer_cred().ok().and_then(|c| c.pid());
		let env = pid.and_then(|pid| get_env(pid).ok());
		let exe = pid.and_then(|pid| fs::read_link(format!("/proc/{}/exe", pid)).ok());
		info!(
			index = index,
			pid,
			exe = exe
				.as_ref()
				.and_then(|exe| exe.to_str().map(|s| s.to_string())),
			"New client connected"
		);

		let (mut messenger_tx, mut messenger_rx) = messenger::create(connection);
		let scenegraph = Arc::new(Scenegraph::default());

		let client = CLIENTS.add(Client {
			event_loop: Arc::downgrade(event_loop),
			index,
			pid,
			env,
			exe,
			stop_notifier: Default::default(),
			join_handle: OnceCell::new(),

			message_sender_handle: Some(messenger_tx.handle()),
			scenegraph: scenegraph.clone(),
			root: OnceCell::new(),
			base_resource_prefixes: Default::default(),
		});
		let _ = client.scenegraph.client.set(Arc::downgrade(&client));
		let _ = client.root.set(Root::create(&client));
		hmd::make_alias(&client);
		spatial::create_interface(&client);
		fields::create_interface(&client);
		drawable::create_interface(&client);
		data::create_interface(&client);
		items::create_interface(&client);
		input::create_interface(&client);
		startup::create_interface(&client);

		if let Some(startup_settings) = client.env.as_ref().and_then(|env| startup_settings(env)) {
			client
				.root
				.get()
				.unwrap()
				.spatial()
				.set_local_transform(startup_settings.transform);
		}

		let _ = client.join_handle.set(tokio::spawn({
			let client = client.clone();
			async move {
				let dispatch_loop = async move {
					loop {
						messenger_rx.dispatch(&*scenegraph).await?
					}
				};
				let flush_loop = async {
					loop {
						messenger_tx.flush().await?
					}
				};

				let result: Result<(), Report> = tokio::select! {
					_ = client.stop_notifier.notified() => Ok(()),
					e = dispatch_loop => e,
					e = flush_loop => e,
				};
				if let Err(e) = &result {
					warn!(error = e.root_cause(), "Client disconnected with error!");
				}
				client.disconnect().await;
				result
			}
		}));
		client
	}

	#[inline]
	pub fn get_node(&self, name: &'static str, path: &str) -> Result<Arc<Node>> {
		self.scenegraph
			.get_node(path)
			.ok_or_else(|| eyre!("{} not found", name))
	}

	pub async fn disconnect(&self) {
		self.stop_notifier.notify_one();
		if let Some(event_loop) = self.event_loop.upgrade() {
			event_loop.clients.lock().await.remove(self.index);
		}
	}
}
impl Drop for Client {
	fn drop(&mut self) {
		self.stop_notifier.notify_one();
		CLIENTS.remove(self);
		info!(
			index = self.index,
			pid = self.pid,
			exe = self
				.exe
				.as_ref()
				.and_then(|exe| exe.to_str().map(|s| s.to_string())),
			"Client disconnected"
		);
	}
}

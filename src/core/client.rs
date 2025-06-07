use super::{
	client_state::{CLIENT_STATES, ClientStateParsed},
	destroy_queue,
	scenegraph::Scenegraph,
};
use crate::{
	core::{registry::OwnedRegistry, task},
	nodes::{
		Node, audio, drawable, fields, input, items,
		root::{ClientState, Root},
		spatial,
	},
};
use color_eyre::eyre::{Result, eyre};
use global_counter::primitive::exact::CounterU32;
use lazy_static::lazy_static;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use stardust_xr::messenger::{self, MessageSenderHandle};
use std::{
	fmt::Debug,
	fs,
	iter::FromIterator,
	path::PathBuf,
	sync::{Arc, OnceLock},
	time::Instant,
};
use tokio::{net::UnixStream, sync::watch, task::JoinHandle};
use tracing::{info, warn};

lazy_static! {
	pub static ref CLIENTS: OwnedRegistry<Client> = OwnedRegistry::new();
	static ref INTERNAL_CLIENT_MESSAGE_TIMES: (watch::Sender<Instant>, watch::Receiver<Instant>) = watch::channel(Instant::now());
	pub static ref INTERNAL_CLIENT: Arc<Client> = CLIENTS.add(Client {
		pid: None,
		// env: None,
		exe: None,

		dispatch_join_handle: OnceLock::new(),
		flush_join_handle: OnceLock::new(),
		disconnect_status: OnceLock::new(),

		id_counter: CounterU32::new(0),
		message_last_received: INTERNAL_CLIENT_MESSAGE_TIMES.1.clone(),
		message_sender_handle: None,
		scenegraph: Default::default(),
		root: OnceLock::new(),
		base_resource_prefixes: Default::default(),
		state: OnceLock::default(),
	});
}
pub fn tick_internal_client() {
	let _ = INTERNAL_CLIENT_MESSAGE_TIMES.0.send(Instant::now());
}

pub fn get_env(pid: i32) -> Result<FxHashMap<String, String>, std::io::Error> {
	let env = fs::read_to_string(format!("/proc/{pid}/environ"))?;
	Ok(FxHashMap::from_iter(
		env.split('\0')
			.filter_map(|var| var.split_once('='))
			.map(|(k, v)| (k.to_string(), v.to_string())),
	))
}
pub fn state(env: &FxHashMap<String, String>) -> Option<Arc<ClientStateParsed>> {
	let token = env.get("STARDUST_STARTUP_TOKEN")?;
	CLIENT_STATES.lock().get(token).cloned()
}

pub struct Client {
	pub pid: Option<i32>,
	// env: Option<FxHashMap<String, String>>,
	exe: Option<PathBuf>,
	dispatch_join_handle: OnceLock<JoinHandle<Result<()>>>,
	flush_join_handle: OnceLock<JoinHandle<Result<()>>>,
	disconnect_status: OnceLock<Result<()>>,

	id_counter: CounterU32,
	message_last_received: watch::Receiver<Instant>,
	pub message_sender_handle: Option<MessageSenderHandle>,
	pub scenegraph: Arc<Scenegraph>,
	pub root: OnceLock<Arc<Root>>,
	pub base_resource_prefixes: Mutex<Vec<PathBuf>>,
	pub state: OnceLock<ClientState>,
}
impl Client {
	pub fn from_connection(connection: UnixStream) -> Result<Arc<Self>> {
		let pid = connection.peer_cred().ok().and_then(|c| c.pid());
		let env = pid.and_then(|pid| get_env(pid).ok());
		let exe = pid.and_then(|pid| fs::read_link(format!("/proc/{}/exe", pid)).ok());
		info!(
			pid,
			exe = exe
				.as_ref()
				.and_then(|exe| exe.to_str().map(|s| s.to_string())),
			"New client connected"
		);

		let (mut messenger_tx, mut messenger_rx) = messenger::create(connection);
		let scenegraph = Arc::new(Scenegraph::default());
		let state = env
			.as_ref()
			.and_then(state)
			.unwrap_or_else(|| Arc::new(ClientStateParsed::default()));

		let (message_time_tx, message_last_received) = watch::channel(Instant::now());
		let client = CLIENTS.add(Client {
			pid,
			// env,
			exe: exe.clone(),

			dispatch_join_handle: OnceLock::new(),
			flush_join_handle: OnceLock::new(),
			disconnect_status: OnceLock::new(),

			id_counter: CounterU32::new(256),
			message_last_received,
			message_sender_handle: Some(messenger_tx.handle()),
			scenegraph: scenegraph.clone(),
			root: OnceLock::new(),
			base_resource_prefixes: Default::default(),
			state: OnceLock::default(),
		});
		let _ = client.scenegraph.client.set(Arc::downgrade(&client));
		let _ = client.root.set(Root::create(&client, state.root)?);
		spatial::create_interface(&client)?;
		fields::create_interface(&client)?;
		drawable::create_interface(&client)?;
		audio::create_interface(&client)?;
		input::create_interface(&client)?;
		items::camera::create_interface(&client)?;
		items::panel::create_interface(&client)?;

		let _ = client.state.set(state.apply_to(&client));

		let pid_printable = pid
			.map(|pid| pid.to_string())
			.unwrap_or_else(|| "??".to_string());
		let exe_printable = exe
			.and_then(|exe| {
				exe.file_name()
					.and_then(|exe| exe.to_str())
					.map(|exe| exe.to_string())
			})
			.unwrap_or_else(|| "??".to_string());
		let _ = client.dispatch_join_handle.get_or_init(|| {
			task::new(
				|| {
					format!(
						"client dispatch pid={} exe={}",
						&pid_printable, &exe_printable,
					)
				},
				{
					let client = client.clone();
					async move {
						loop {
							if let Err(e) = messenger_rx.dispatch(&*scenegraph).await {
								client.disconnect(Err(e.into()));
							}
							let _ = message_time_tx.send(Instant::now());
						}
					}
				},
			)
			.unwrap()
		});
		let _ = client.flush_join_handle.get_or_init(|| {
			task::new(
				|| format!("client flush pid={} exe={}", &pid_printable, &exe_printable,),
				{
					let client = client.clone();
					async move {
						loop {
							if let Err(e) = messenger_tx.flush().await {
								client.disconnect(Err(e.into()));
							}
						}
					}
				},
			)
			.unwrap()
		});

		Ok(client)
	}

	pub fn get_cmdline(&self) -> Option<Vec<String>> {
		let pid = self.pid?;
		let exe_proc_path = format!("/proc/{pid}/exe");
		let cmdline_proc_path = format!("/proc/{pid}/cmdline");
		let exe = std::fs::read_link(exe_proc_path).ok()?;
		let cmdline = std::fs::read_to_string(cmdline_proc_path).ok()?;
		let mut cmdline_split: Vec<_> = cmdline.split('\0').map(ToString::to_string).collect();
		cmdline_split.pop();
		*cmdline_split.get_mut(0).unwrap() = exe.to_str()?.to_string();
		Some(cmdline_split)
	}
	pub fn get_cwd(&self) -> Option<PathBuf> {
		let pid = self.pid?;
		let cwd_proc_path = format!("/proc/{pid}/cwd");
		std::fs::read_link(cwd_proc_path).ok()
	}
	pub async fn save_state(&self) -> Option<ClientStateParsed> {
		println!("start save state");
		let internal = self.root.get()?.save_state().await.ok()?;
		println!("finished save state");
		Some(ClientStateParsed::from_deserialized(self, internal))
	}

	pub fn generate_id(&self) -> u64 {
		self.id_counter.inc() as u64
	}

	#[inline]
	pub fn get_node(&self, name: &'static str, id: u64) -> Result<Arc<Node>> {
		self.scenegraph
			.get_node(id)
			.ok_or_else(|| eyre!("{} not found", name))
	}

	pub fn unresponsive(&self) -> bool {
		let time_since_last_message = self.message_last_received.borrow().elapsed();
		time_since_last_message.as_millis() > 500
	}

	pub fn disconnect(&self, reason: Result<()>) {
		let _ = self.disconnect_status.set(reason);
		if let Some(dispatch_join_handle) = self.dispatch_join_handle.get() {
			dispatch_join_handle.abort();
		}
		if let Some(flush_join_handle) = self.flush_join_handle.get() {
			flush_join_handle.abort();
		}
		if let Some(client) = CLIENTS.remove(self) {
			destroy_queue::add(client);
		}
	}
}
impl Debug for Client {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Client")
			.field("pid", &self.pid)
			.field("exe", &self.exe)
			.field("dispatch_join_handle", &self.dispatch_join_handle)
			.field("flush_join_handle", &self.flush_join_handle)
			.field("disconnect_status", &self.disconnect_status)
			.field("base_resource_prefixes", &self.base_resource_prefixes)
			.field("state", &self.state)
			.finish()
	}
}
impl Drop for Client {
	fn drop(&mut self) {
		info!(
			pid = self.pid,
			exe = self
				.exe
				.as_ref()
				.and_then(|exe| exe.to_str().map(|s| s.to_string())),
			disconnect_status = match self.disconnect_status.take() {
				Some(Ok(_)) => "Graceful disconnect".to_string(),
				Some(Err(e)) => format!("Error: {}", e.root_cause()),
				None => "Unknown".to_string(),
			},
			"Client disconnected"
		);
	}
}

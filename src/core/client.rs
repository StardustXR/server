use super::client_state::{CLIENT_STATES, ClientStateParsed};
use crate::{
	PION, core::{Id, registry::OwnedRegistry}, impl_transaction_handler, nodes::{audio, drawable, fields, spatial}
};
use binderbinder::TransactionHandler;
use color_eyre::eyre::Result;
use global_counter::primitive::exact::CounterU32;
use gluon_wire::{GluonCtx, GluonDataBuilder, GluonDataReader, drop_tracking::DropNotifier};
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use rustix::process::RawPid;
use stardust_xr_protocol::protocol::{
	audio::AudioInterface,
	client::{Client, ClientState},
	dmatex::DmatexInterface,
	field::FieldInterface,
	lines::LinesInterface,
	model::ModelInterface,
	server::ServerHandler,
	sky::SkyInterface,
	spatial::SpatialInterface,
	spatial_query::SpatialQueryInterface,
	text::TextInterface,
};
use std::{
	fmt::Debug,
	fs,
	iter::FromIterator,
	path::PathBuf,
	sync::{Arc, LazyLock, OnceLock},
	time::Instant,
};
use tokio::sync::{RwLock, watch};
use tracing::info;

pub static CLIENTS: OwnedRegistry<ConnectedClient> = OwnedRegistry::new();

static INTERNAL_CLIENT_MESSAGE_TIMES: LazyLock<(watch::Sender<Instant>, watch::Receiver<Instant>)> =
	LazyLock::new(|| watch::channel(Instant::now()));
pub static INTERNAL_CLIENT: LazyLock<Arc<ConnectedClient>> = LazyLock::new(|| {
	CLIENTS.add(ConnectedClient {
		pid: None,
		// env: None,
		exe: None,

		disconnect_status: OnceLock::new(),

		id_counter: CounterU32::new(0),
		base_resource_prefixes: Default::default(),
		state: OnceLock::default(),
		drop_notifs: Default::default(),
		client: todo!(),
	})
});
pub fn tick_internal_client() {
	let _ = INTERNAL_CLIENT_MESSAGE_TIMES.0.send(Instant::now());
}

pub fn get_env(pid: RawPid) -> Result<FxHashMap<String, String>, std::io::Error> {
	let env = fs::read_to_string(format!("/proc/{pid}/environ"))?;
	Ok(FxHashMap::from_iter(
		env.split('\0')
			.filter_map(|var| var.split_once('='))
			.map(|(k, v)| (k.to_string(), v.to_string())),
	))
}
pub fn state(env: &FxHashMap<String, String>) -> Option<Arc<ClientStateParsed>> {
	let token = env.get("STARDUST_STARTUP_TOKEN")?;
	CLIENT_STATES.get(token).as_deref().cloned()
}

pub struct ConnectedClient {
	pub pid: Option<i32>,
	client: Client,
	// env: Option<FxHashMap<String, String>>,
	exe: Option<PathBuf>,
	disconnect_status: OnceLock<Result<()>>,

	id_counter: CounterU32,
	pub base_resource_prefixes: Arc<Vec<PathBuf>>,
	pub state: OnceLock<ClientState>,
	drop_notifs: RwLock<Vec<DropNotifier>>,
}
impl ConnectedClient {
	pub async fn from_connection(
		client: Client,
		pid: RawPid,
		base_resource_prefixes: Vec<PathBuf>,
	) -> Result<Arc<Self>> {
		let env = get_env(pid).ok();
		let exe = fs::read_link(format!("/proc/{pid}/exe")).ok();
		info!(
			pid,
			exe = exe
				.as_ref()
				.and_then(|exe| exe.to_str().map(|s| s.to_string())),
			"New client connected"
		);

		let state = env
			.as_ref()
			.and_then(state)
			.unwrap_or_else(|| Arc::new(ClientStateParsed::default()));

		let client = PION.register_object(ConnectedClient {
			pid,
			// env,
			exe: exe.clone(),

			disconnect_status: OnceLock::new(),

			id_counter: CounterU32::new(256),
			base_resource_prefixes: Default::default(),
			state: OnceLock::default(),
			drop_notifs: Default::default(),
			client,
		});
		CLIENTS.add_raw(&client);
		let _ = client.scenegraph.client.set(Arc::downgrade(&client));

		let _ = client.state.set(state.apply_to(&client).await);

		Ok(client)
	}

	pub fn get_cmdline(&self) -> Option<Vec<String>> {
		let pid = self.pid;
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
		info!("start save state");
		let internal = self.root.get()?.save_state().await.ok()?;
		info!("finished save state");
		Some(ClientStateParsed::from_deserialized(self, internal))
	}

	pub fn generate_id(&self) -> Id {
		Id(self.id_counter.inc() as u64)
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
		CLIENTS.remove(self);
	}
}
impl Debug for ConnectedClient {
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
impl ServerHandler for ConnectedClient {
	async fn spatial_interface(&self, _ctx: GluonCtx) -> SpatialInterface {
		todo!()
	}

	async fn field_interface(&self, _ctx: GluonCtx) -> FieldInterface {
		todo!()
	}

	async fn dmatex_interface(&self, _ctx: GluonCtx) -> DmatexInterface {
		todo!()
	}

	async fn text_interface(&self, _ctx: GluonCtx) -> TextInterface {
		todo!()
	}

	async fn model_interface(&self, _ctx: GluonCtx) -> ModelInterface {
		todo!()
	}

	async fn lines_interface(&self, _ctx: GluonCtx) -> LinesInterface {
		todo!()
	}

	async fn sky_interface(&self, _ctx: GluonCtx) -> SkyInterface {
		todo!()
	}

	async fn audio_interface(&self, _ctx: GluonCtx) -> AudioInterface {
		todo!()
	}

	async fn spatial_query_interface(&self, _ctx: GluonCtx) -> SpatialQueryInterface {
		todo!()
	}

	async fn generate_state_token(&self, _ctx: GluonCtx, state: ClientState) -> String {
		ClientStateParsed::from_deserialized(self, state).token()
	}

	async fn drop_notification_requested(&self, notifier: DropNotifier) {
		self.drop_notifs.write().await.push(notifier);
	}
}
impl Drop for ConnectedClient {
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
impl_transaction_handler!(ConnectedClient);

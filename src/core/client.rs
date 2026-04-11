use super::client_state::{CLIENT_STATES, ClientStateParsed};
use crate::{
	PION,
	core::registry::OwnedRegistry,
	nodes::{
		audio::AudioInterface,
		drawable::{
			dmatex::DmatexInterface, lines::LinesInterface, model::ModelInterface,
			sky::SkyInterface, text::TextInterface,
		},
		fields::FieldInterface,
		spatial::SpatialInterface,
	},
};
use bevy::prelude::Deref;
use binderbinder::binder_object::{BinderObject, ToBinderObjectOrRef};
use color_eyre::eyre::Result;
use global_counter::primitive::exact::CounterU32;
use gluon_wire::{GluonCtx, impl_transaction_handler};
use rustc_hash::FxHashMap;
use rustix::process::RawPid;
use stardust_xr_protocol::{
	audio::AudioInterface as AudioInterfaceProxy,
	client::{Client, ClientState},
	dmatex::DmatexInterface as DmatexInterfaceProxy,
	field::FieldInterface as FieldInterfaceProxy,
	lines::LinesInterface as LinesInterfaceProxy,
	model::ModelInterface as ModelInterfaceProxy,
	server::ServerHandler,
	sky::SkyInterface as SkyInterfaceProxy,
	spatial::SpatialInterface as SpatialInterfaceProxy,
	spatial_query::SpatialQueryInterface as SpatialQueryInterfaceProxy,
	text::TextInterface as TextInterfaceProxy,
};
use std::{
	fmt::Debug,
	fs,
	iter::FromIterator,
	path::PathBuf,
	sync::{Arc, OnceLock},
};
use tracing::info;

pub static CLIENTS: OwnedRegistry<BinderObject<ConnectedClient>> = OwnedRegistry::new();

// static INTERNAL_CLIENT_MESSAGE_TIMES: LazyLock<(watch::Sender<Instant>, watch::Receiver<Instant>)> =
// LazyLock::new(|| watch::channel(Instant::now()));
// pub static INTERNAL_CLIENT: LazyLock<Arc<ConnectedClient>> = LazyLock::new(|| {
// 	CLIENTS.add(ConnectedClient {
// 		pid: None,
// 		// env: None,
// 		exe: None,
//
// 		disconnect_status: OnceLock::new(),
//
// 		id_counter: CounterU32::new(0),
// 		base_resource_prefixes: Default::default(),
// 		state: OnceLock::default(),
// 		drop_notifs: Default::default(),
// 		client: todo!(),
// 	})
// });
// pub fn tick_internal_client() {
// 	let _ = INTERNAL_CLIENT_MESSAGE_TIMES.0.send(Instant::now());
// }

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

#[derive(Debug, Deref)]
pub struct ConnectedClient {
	pub pid: RawPid,
	#[deref]
	client: Client,
	exe: Option<PathBuf>,
	disconnect_status: OnceLock<Result<()>>,

	id_counter: CounterU32,
	pub base_resource_prefixes: Arc<Vec<PathBuf>>,

	spatial_interface: SpatialInterfaceProxy,
	field_interface: FieldInterfaceProxy,
	dmatex_interface: DmatexInterfaceProxy,
	text_interface: TextInterfaceProxy,
	model_interface: ModelInterfaceProxy,
	lines_interface: LinesInterfaceProxy,
	sky_interface: SkyInterfaceProxy,
	audio_interface: AudioInterfaceProxy,
	// spatial_query_interface: SpatialQueryInterfaceProxy,
}
impl ConnectedClient {
	pub fn from_connection(
		client: Client,
		pid: RawPid,
		base_resource_prefixes: Vec<PathBuf>,
	) -> Result<(Arc<BinderObject<Self>>, ClientState)> {
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

		let p = Arc::new(base_resource_prefixes);
		let client = PION.register_object(ConnectedClient {
			pid,
			// env,
			exe: exe.clone(),

			disconnect_status: OnceLock::new(),

			id_counter: CounterU32::new(256),
			base_resource_prefixes: p.clone(),
			client,

			spatial_interface: SpatialInterfaceProxy::from_handler(&SpatialInterface::new(&p)),
			field_interface: FieldInterfaceProxy::from_handler(&FieldInterface::new(&p)),
			dmatex_interface: DmatexInterfaceProxy::from_handler(&DmatexInterface::new(&p)),
			text_interface: TextInterfaceProxy::from_handler(&TextInterface::new(&p)),
			model_interface: ModelInterfaceProxy::from_handler(&ModelInterface::new(&p)),
			lines_interface: LinesInterfaceProxy::from_handler(&LinesInterface::new(&p)),
			sky_interface: SkyInterfaceProxy::from_handler(&SkyInterface::new(&p)),
			audio_interface: AudioInterfaceProxy::from_handler(&AudioInterface::new(&p)),
			// spatial_query_interface: SpatialQueryInterfaceProxy::from_handler(&SpatialQueryInterface::new(&p)),
		});
		CLIENTS.add_raw(client.clone());
		let death_future = client.client.death_or_drop();
		// TODO: make sure this is cleaned up if we ever have a reason for disconnect that isn't the
		// client being destroyed
		tokio::spawn({
			let client = Arc::downgrade(&client);
			async move {
				death_future.await;
				if let Some(client) = client.upgrade() {
					client.disconnect(Ok(()));
				}
			}
		});

		Ok((client, state.apply()))
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
		let pid = self.pid;
		let cwd_proc_path = format!("/proc/{pid}/cwd");
		std::fs::read_link(cwd_proc_path).ok()
	}
	pub async fn save_state(&self) -> Option<ClientStateParsed> {
		info!("start save state");
		let internal = self.client.get_state().await.ok()?;
		info!("finished save state");
		Some(ClientStateParsed::from_deserialized(self, internal))
	}

	pub fn unresponsive(&self) -> bool {
		// TODO: reimplement this somehow, probably either based in ping or the binder freeze stuff
		// let time_since_last_message = self.message_last_received.borrow().elapsed();
		// time_since_last_message.as_millis() > 500
		false
	}
}
pub trait ConnectedClientExt {
	fn disconnect(&self, reason: Result<()>);
}
impl ConnectedClientExt for BinderObject<ConnectedClient> {
	fn disconnect(&self, reason: Result<()>) {
		let _ = self.disconnect_status.set(reason);
		CLIENTS.remove(self);
	}
}

impl ServerHandler for ConnectedClient {
	async fn spatial_interface(&self, _ctx: GluonCtx) -> SpatialInterfaceProxy {
		self.spatial_interface.clone()
	}

	async fn field_interface(&self, _ctx: GluonCtx) -> FieldInterfaceProxy {
		self.field_interface.clone()
	}

	async fn dmatex_interface(&self, _ctx: GluonCtx) -> DmatexInterfaceProxy {
		self.dmatex_interface.clone()
	}

	async fn text_interface(&self, _ctx: GluonCtx) -> TextInterfaceProxy {
		self.text_interface.clone()
	}

	async fn model_interface(&self, _ctx: GluonCtx) -> ModelInterfaceProxy {
		self.model_interface.clone()
	}

	async fn lines_interface(&self, _ctx: GluonCtx) -> LinesInterfaceProxy {
		self.lines_interface.clone()
	}

	async fn sky_interface(&self, _ctx: GluonCtx) -> SkyInterfaceProxy {
		self.sky_interface.clone()
	}

	async fn audio_interface(&self, _ctx: GluonCtx) -> AudioInterfaceProxy {
		self.audio_interface.clone()
	}

	async fn spatial_query_interface(&self, _ctx: GluonCtx) -> SpatialQueryInterfaceProxy {
		// TODO: use the protper thingy here
		SpatialQueryInterfaceProxy::from_object_or_ref(
			self.audio_interface.to_binder_object_or_ref(),
		)
	}

	async fn generate_state_token(&self, _ctx: GluonCtx, state: ClientState) -> String {
		ClientStateParsed::from_deserialized(self, state).token()
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

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
	query::{QueryInterface, spatial_query::SpatialQueryInterface},
};
use color_eyre::eyre::Result;
use global_counter::primitive::exact::CounterU32;
use gluon::{Handler, ObjectRef};
use rustc_hash::FxHashMap;
use rustix::process::RawPid;
use stardust_xr_protocol::{
	audio::AudioInterface as AudioInterfaceProxy,
	client::{Client, FrameInfo},
	dmatex::DmatexInterface as DmatexInterfaceProxy,
	field::FieldInterface as FieldInterfaceProxy,
	lines::LinesInterface as LinesInterfaceProxy,
	model::ModelInterface as ModelInterfaceProxy,
	query::QueryInterface as QueryInterfaceProxy,
	server::ServerHandler,
	sky::SkyInterface as SkyInterfaceProxy,
	spatial::{SpatialInterface as SpatialInterfaceProxy, SpatialRef},
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

pub static CLIENTS: OwnedRegistry<ConnectedClient> = OwnedRegistry::new();

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
pub fn state(token: &String) -> Option<Arc<ClientStateParsed>> {
	CLIENT_STATES.get(token).as_deref().cloned()
}

#[derive(Debug, Handler)]
pub struct ConnectedClient {
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
	query_interface: QueryInterfaceProxy,
	spatial_query_interface: SpatialQueryInterfaceProxy,
}
impl ConnectedClient {
	pub fn from_connection(
		client: Client,
		// pid: RawPid,
		startup_token: Option<String>,
		base_resource_prefixes: Vec<PathBuf>,
	) -> Result<(ObjectRef<Self>, SpatialRef)> {
		// let env = get_env(pid).ok();
		// let exe = fs::read_link(format!("/proc/{pid}/exe")).ok();
		let exe = None;
		info!("New client connected");

		let state = startup_token
			.as_ref()
			.and_then(state)
			.unwrap_or_else(|| Arc::new(ClientStateParsed::default()));

		let p = Arc::new(base_resource_prefixes);

		let spatial_interface =
			SpatialInterfaceProxy::from_handler(&SpatialInterface::new(&p).to_service());
		let field_interface =
			FieldInterfaceProxy::from_handler(&FieldInterface::new(&p).to_service());
		let dmatex_interface =
			DmatexInterfaceProxy::from_handler(&DmatexInterface::new(&p).to_service());
		let text_interface = TextInterfaceProxy::from_handler(&TextInterface::new(&p).to_service());
		let model_interface =
			ModelInterfaceProxy::from_handler(&ModelInterface::new(&p).to_service());
		let lines_interface =
			LinesInterfaceProxy::from_handler(&LinesInterface::new(&p).to_service());
		let sky_interface = SkyInterfaceProxy::from_handler(&SkyInterface::new(&p).to_service());
		let audio_interface =
			AudioInterfaceProxy::from_handler(&AudioInterface::new(&p).to_service());
		let query_interface =
			QueryInterfaceProxy::from_handler(&QueryInterface::new(&p).to_service());
		let spatial_query_interface =
			SpatialQueryInterfaceProxy::from_handler(&SpatialQueryInterface::new(&p).to_service());

		let client = PION.register_object(ConnectedClient {
			// env,
			exe: exe.clone(),

			disconnect_status: OnceLock::new(),

			id_counter: CounterU32::new(256),
			base_resource_prefixes: p.clone(),
			client,

			spatial_interface,
			field_interface,
			dmatex_interface,
			text_interface,
			model_interface,
			lines_interface,
			sky_interface,
			audio_interface,
			query_interface,
			spatial_query_interface,
		});
		let death_future = client.strong_refs_hit_zero();
		let client = client;
		CLIENTS.add_raw(client.handler_arc().clone());
		// TODO: make sure this is cleaned up if we ever have a reason for disconnect that isn't the
		// client being destroyed
		tokio::spawn({
			let client = Arc::downgrade(client.handler_arc());
			async move {
				death_future.await;
				if let Some(client) = client.upgrade() {
					client.disconnect(Ok(()));
				}
			}
		});

		Ok((client.to_service(), state.apply()))
	}

	pub fn get_cmdline(&self) -> Option<Vec<String>> {
		None
		// let pid = self.pid;
		// let exe_proc_path = format!("/proc/{pid}/exe");
		// let cmdline_proc_path = format!("/proc/{pid}/cmdline");
		// let exe = std::fs::read_link(exe_proc_path).ok()?;
		// let cmdline = std::fs::read_to_string(cmdline_proc_path).ok()?;
		// let mut cmdline_split: Vec<_> = cmdline.split('\0').map(ToString::to_string).collect();
		// cmdline_split.pop();
		// *cmdline_split.get_mut(0).unwrap() = exe.to_str()?.to_string();
		// Some(cmdline_split)
	}
	pub fn get_cwd(&self) -> Option<PathBuf> {
		None
		// let pid = self.pid;
		// let cwd_proc_path = format!("/proc/{pid}/cwd");
		// std::fs::read_link(cwd_proc_path).ok()
	}

	pub fn unresponsive(&self) -> bool {
		// TODO: reimplement this somehow, probably either based in ping or the binder freeze stuff
		// let time_since_last_message = self.message_last_received.borrow().elapsed();
		// time_since_last_message.as_millis() > 500
		false
	}

	pub fn frame(&self, info: FrameInfo) {
		_ = self.client.frame(info);
	}

	fn disconnect(self: &Arc<Self>, reason: Result<()>) {
		let _ = self.disconnect_status.set(reason);
		CLIENTS.remove(self);
	}
}

impl ServerHandler for ConnectedClient {
	async fn spatial_interface(&self, _ctx: gluon::Context) -> SpatialInterfaceProxy {
		self.spatial_interface.clone()
	}

	async fn field_interface(&self, _ctx: gluon::Context) -> FieldInterfaceProxy {
		self.field_interface.clone()
	}

	async fn dmatex_interface(&self, _ctx: gluon::Context) -> DmatexInterfaceProxy {
		self.dmatex_interface.clone()
	}

	async fn text_interface(&self, _ctx: gluon::Context) -> TextInterfaceProxy {
		self.text_interface.clone()
	}

	async fn model_interface(&self, _ctx: gluon::Context) -> ModelInterfaceProxy {
		self.model_interface.clone()
	}

	async fn lines_interface(&self, _ctx: gluon::Context) -> LinesInterfaceProxy {
		self.lines_interface.clone()
	}

	async fn sky_interface(&self, _ctx: gluon::Context) -> SkyInterfaceProxy {
		self.sky_interface.clone()
	}

	async fn audio_interface(&self, _ctx: gluon::Context) -> AudioInterfaceProxy {
		self.audio_interface.clone()
	}

	async fn query_interface(&self, _ctx: gluon::Context) -> QueryInterfaceProxy {
		self.query_interface.clone()
	}

	async fn spatial_query_interface(&self, _ctx: gluon::Context) -> SpatialQueryInterfaceProxy {
		self.spatial_query_interface.clone()
	}

	async fn generate_startup_token(&self, _ctx: gluon::Context, root: SpatialRef) -> String {
		ClientStateParsed::from_deserialized(self, &root).token()
	}
}
impl Drop for ConnectedClient {
	fn drop(&mut self) {
		info!(
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

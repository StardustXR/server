use super::client_state::{CLIENT_STATES, ClientStateParsed};
use crate::{
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
use gluon::{Handler, NodeError, RefExt};
use parking_lot::RwLock;
use stardust_xr_protocol::{
	audio::AudioInterface as AudioInterfaceProxy,
	client::{Client, FrameInfo},
	dmatex::DmatexInterface as DmatexInterfaceProxy,
	field::FieldInterface as FieldInterfaceProxy,
	lines::LinesInterface as LinesInterfaceProxy,
	model::ModelInterface as ModelInterfaceProxy,
	query::QueryInterface as QueryInterfaceProxy,
	server::{Server, ServerHandler},
	sky::SkyInterface as SkyInterfaceProxy,
	spatial::{SpatialInterface as SpatialInterfaceProxy, SpatialRef},
	spatial_query::SpatialQueryInterface as SpatialQueryInterfaceProxy,
	text::TextInterface as TextInterfaceProxy,
	types::CreateError,
};
use stardust_xr_server_foundation::registry::Registry;
use std::{
	fmt::Debug,
	path::PathBuf,
	sync::{Arc, OnceLock},
};
use tracing::info;

pub static CLIENTS: Registry<ConnectedClient> = Registry::new();

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

// pub fn get_env(pid: RawPid) -> Result<FxHashMap<String, String>, std::io::Error> {
// 	let env = fs::read_to_string(format!("/proc/{pid}/environ"))?;
// 	Ok(FxHashMap::from_iter(
// 		env.split('\0')
// 			.filter_map(|var| var.split_once('='))
// 			.map(|(k, v)| (k.to_string(), v.to_string())),
// 	))
// }
pub fn state(token: &String) -> Option<Arc<ClientStateParsed>> {
	CLIENT_STATES.get(token).as_deref().cloned()
}

#[derive(Debug, Handler)]
pub struct ConnectedClient {
	client: RwLock<Option<Client>>,
	exe: Option<PathBuf>,
	disconnect_status: OnceLock<Result<()>>,

	_id_counter: CounterU32,
	pub _base_resource_prefixes: Arc<Vec<PathBuf>>,

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
	) -> Result<(Server, SpatialRef), NodeError> {
		// let env = get_env(pid).ok();
		// let exe = fs::read_link(format!("/proc/{pid}/exe")).ok();
		let exe = None;
		info!("New client connected");

		let state = startup_token
			.as_ref()
			.and_then(state)
			.unwrap_or_else(|| Arc::new(ClientStateParsed::default()));

		let p = Arc::new(base_resource_prefixes);

		let spatial_interface = SpatialInterfaceProxy::new_service(SpatialInterface::new(&p))?;
		let field_interface = FieldInterfaceProxy::new_service(FieldInterface::new(&p))?;
		let dmatex_interface = DmatexInterfaceProxy::new_service(DmatexInterface::new(&p))?;
		let text_interface = TextInterfaceProxy::new_service(TextInterface::new(&p))?;
		let model_interface = ModelInterfaceProxy::new_service(ModelInterface::new(&p))?;
		let lines_interface = LinesInterfaceProxy::new_service(LinesInterface::new(&p))?;
		let sky_interface = SkyInterfaceProxy::new_service(SkyInterface::new(&p))?;
		let audio_interface = AudioInterfaceProxy::new_service(AudioInterface::new(&p))?;
		let query_interface = QueryInterfaceProxy::new_service(QueryInterface::new(&p))?;
		let spatial_query_interface =
			SpatialQueryInterfaceProxy::new_service(SpatialQueryInterface::new(&p))?;

		let server_handler = Arc::new(ConnectedClient {
			// env,
			exe: exe.clone(),

			disconnect_status: OnceLock::new(),

			_id_counter: CounterU32::new(256),
			_base_resource_prefixes: p.clone(),
			client: RwLock::new(Some(client)),

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
		CLIENTS.add_raw(&server_handler);
		let server = Server::new_service(server_handler)?;

		Ok((server, state.apply()))
	}

	pub fn frame(&self, info: FrameInfo) {
		if let Some(client) = self.client.read().as_ref() {
			_ = client.frame(info);
		}
	}

	fn disconnect(&self, reason: Result<()>) {
		let _ = self.disconnect_status.set(reason);
		self.client.write().take();
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

	async fn generate_startup_token(
		&self,
		_ctx: gluon::Context,
		root: SpatialRef,
	) -> Result<String, CreateError> {
		Ok(ClientStateParsed::from_deserialized(self, &root)?.token())
	}
}
impl Drop for ConnectedClient {
	fn drop(&mut self) {
		CLIENTS.remove(self);
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

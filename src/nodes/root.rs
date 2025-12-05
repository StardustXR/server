use super::spatial::Spatial;
use super::{Aspect, AspectIdentifier, Node};
use crate::core::Id;
use crate::core::client::{CLIENTS, Client};
use crate::core::client_state::ClientStateParsed;
use crate::core::error::Result;
use crate::nodes::spatial::SPATIAL_REF_ASPECT_ALIAS_INFO;
use crate::session::connection_env;
use glam::Mat4;
use stardust_xr_server_foundation::bail;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tracing::info;

stardust_xr_server_codegen::codegen_root_protocol!();

pub struct Root {
	node: Arc<Node>,
	connect_instant: Instant,
}
impl Root {
	pub fn create(client: &Arc<Client>, transform: Mat4) -> Result<Arc<Self>> {
		let node = Node::from_id(client, Id(0), false);
		let node = node.add_to_scenegraph()?;
		let _ = Spatial::add_to(&node, None, transform);
		let root_aspect = node.add_aspect(Root {
			node: node.clone(),
			connect_instant: Instant::now(),
		});
		Ok(root_aspect)
	}

	pub fn send_frame_events(delta: f64) {
		for client in CLIENTS.get_vec() {
			let Some(root) = client.root.get() else {
				continue;
			};
			let _ = root_client::frame(
				&root.node,
				&FrameInfo {
					delta: delta as f32,
					elapsed: root.connect_instant.elapsed().as_secs_f32(),
				},
			);
		}
	}

	pub fn set_transform(&self, transform: Mat4) {
		let spatial = self.node.get_aspect::<Spatial>().unwrap();
		// spatial.set_spatial_parent(None).unwrap();
		spatial.set_local_transform(transform);
	}
	pub async fn save_state(&self) -> Result<ClientState> {
		Ok(root_client::save_state(&self.node).await?.0)
	}
}
impl AspectIdentifier for Root {
	impl_aspect_for_root_aspect_id! {}
}
impl Aspect for Root {
	impl_aspect_for_root_aspect! {}
}
impl RootAspect for Root {
	async fn get_state(_node: Arc<Node>, calling_client: Arc<Client>) -> Result<ClientState> {
		let Some(state) = calling_client.state.get() else {
			bail!("Couldn't get state");
		};
		Ok(state.clone())
	}

	#[doc = "Get a hashmap of all the environment variables to connect a given app to the stardust server"]
	async fn get_connection_environment(
		_node: Arc<Node>,
		_calling_client: Arc<Client>,
	) -> Result<stardust_xr_wire::values::Map<String, String>> {
		Ok(connection_env())
	}

	#[doc = "Generate a client state token and return it back.\n\n When launching a new client, set the environment variable `STARDUST_STARTUP_TOKEN` to the returned string.\n Make sure the environment variable shows in `/proc/{pid}/environ` as that's the only reliable way to pass the value to the server (suggestions welcome).\n"]
	async fn generate_state_token(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		state: ClientState,
	) -> Result<String> {
		Ok(ClientStateParsed::from_deserialized(&calling_client, state).token())
	}

	#[doc = "Set initial list of folders to look for namespaced resources in"]
	fn set_base_prefixes(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		prefixes: Vec<String>,
	) -> Result<()> {
		info!(?calling_client, ?prefixes, "Set base prefixes");
		*calling_client.base_resource_prefixes.lock() =
			prefixes.into_iter().map(PathBuf::from).collect();
		Ok(())
	}

	#[doc = "Cleanly disconnect from the server"]
	fn disconnect(_node: Arc<Node>, calling_client: Arc<Client>) -> Result<()> {
		calling_client.disconnect(Ok(()));
		Ok(())
	}
}

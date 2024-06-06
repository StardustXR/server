use super::spatial::Spatial;
use super::Node;
use crate::core::client::Client;
use crate::core::client_state::ClientStateParsed;
use crate::core::registry::Registry;
#[cfg(feature = "wayland")]
use crate::wayland::WAYLAND_DISPLAY;
#[cfg(feature = "xwayland")]
use crate::wayland::X_DISPLAY;
use crate::STARDUST_INSTANCE;
use color_eyre::eyre::{bail, Result};
use glam::Mat4;
use rustc_hash::FxHashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;

static ROOT_REGISTRY: Registry<Root> = Registry::new();

stardust_xr_server_codegen::codegen_root_protocol!();

pub struct Root(Arc<Node>);
impl Root {
	pub fn create(client: &Arc<Client>, transform: Mat4) -> Result<Arc<Self>> {
		let node = Node::from_id(client, 0, false);
		<Self as RootAspect>::add_node_members(&node);
		let node = node.add_to_scenegraph()?;
		let _ = Spatial::add_to(&node, None, transform, false);

		Ok(ROOT_REGISTRY.add(Root(node)))
	}

	pub fn send_frame_events(delta: f64) {
		let info = FrameInfo {
			delta: delta as f32,
			elapsed: 0.0,
		};
		for root in ROOT_REGISTRY.get_valid_contents() {
			let _ = root_client::frame(&root.0, &info);
		}
	}

	pub fn set_transform(&self, transform: Mat4) {
		let spatial = self.0.get_aspect::<Spatial>().unwrap();
		spatial.set_spatial_parent(None).unwrap();
		spatial.set_local_transform(transform);
	}
	pub async fn save_state(&self) -> Result<ClientState> {
		Ok(root_client::save_state(&self.0).await?.0)
	}
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
	) -> Result<stardust_xr::values::Map<String, String>> {
		macro_rules! var_env_insert {
			($env:ident, $name:ident) => {
				$env.insert(stringify!($name).to_string(), $name.get().unwrap().clone());
			};
		}

		let mut env: FxHashMap<String, String> = FxHashMap::default();
		var_env_insert!(env, STARDUST_INSTANCE);
		#[cfg(feature = "wayland")]
		{
			var_env_insert!(env, WAYLAND_DISPLAY);
			#[cfg(feature = "xwayland")]
			env.insert(
				"DISPLAY".to_string(),
				format!(":{}", X_DISPLAY.get().unwrap()),
			);
			env.insert("GDK_BACKEND".to_string(), "wayland".to_string());
			env.insert("QT_QPA_PLATFORM".to_string(), "wayland".to_string());
			env.insert("MOZ_ENABLE_WAYLAND".to_string(), "1".to_string());
			env.insert("CLUTTER_BACKEND".to_string(), "wayland".to_string());
			env.insert("SDL_VIDEODRIVER".to_string(), "wayland".to_string());
		}

		Ok(env)
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
	fn disconnect(_node: Arc<Node>, calling_client: Arc<Client>) -> color_eyre::eyre::Result<()> {
		calling_client.disconnect(Ok(()));
		Ok(())
	}
}
impl Drop for Root {
	fn drop(&mut self) {
		ROOT_REGISTRY.remove(self);
	}
}

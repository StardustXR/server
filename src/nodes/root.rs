use super::spatial::Spatial;
use super::{Message, Node};
use crate::core::client::Client;
use crate::core::client_state::{ClientState, ClientStateInternal};
use crate::core::registry::Registry;
use crate::core::scenegraph::MethodResponseSender;
#[cfg(feature = "wayland")]
use crate::wayland::WAYLAND_DISPLAY;
#[cfg(feature = "xwayland")]
use crate::wayland::X_DISPLAY;
use crate::STARDUST_INSTANCE;
use color_eyre::eyre::Result;
use glam::Mat4;
use rustc_hash::FxHashMap;
use stardust_xr::schemas::flex::{deserialize, serialize};
use tracing::instrument;

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

static ROOT_REGISTRY: Registry<Root> = Registry::new();

pub struct Root {
	pub node: Arc<Node>,
	send_frame_event: AtomicBool,
}
impl Root {
	pub fn create(client: &Arc<Client>) -> Result<Arc<Self>> {
		let node = Node::create(client, "", "", false);
		node.add_local_signal("subscribe_frame", Root::subscribe_frame_flex);
		node.add_local_signal("set_base_prefixes", Root::set_base_prefixes_flex);
		node.add_local_method("state_token", Root::state_token_flex);
		node.add_local_method(
			"get_connection_environment",
			get_connection_environment_flex,
		);
		let node = node.add_to_scenegraph()?;
		let _ = Spatial::add_to(&node, None, client.state.root, false);

		Ok(ROOT_REGISTRY.add(Root {
			node,
			send_frame_event: AtomicBool::from(false),
		}))
	}

	fn subscribe_frame_flex(
		_node: &Node,
		calling_client: Arc<Client>,
		_message: Message,
	) -> Result<()> {
		calling_client
			.root
			.get()
			.unwrap()
			.send_frame_event
			.store(true, Ordering::Relaxed);
		Ok(())
	}

	#[instrument(level = "debug")]
	pub fn send_frame_events(delta: f64) {
		if let Ok(data) = serialize((delta, 0.0)) {
			for root in ROOT_REGISTRY.get_valid_contents() {
				if root.send_frame_event.load(Ordering::Relaxed) {
					let _ = root.node.send_remote_signal("frame", data.clone());
				}
			}
		}
	}

	fn set_base_prefixes_flex(
		_node: &Node,
		calling_client: Arc<Client>,
		message: Message,
	) -> Result<()> {
		*calling_client.base_resource_prefixes.lock() = deserialize(message.as_ref())?;
		Ok(())
	}

	fn state_token_flex(
		_node: &Node,
		calling_client: Arc<Client>,
		message: Message,
		response: MethodResponseSender,
	) {
		response.wrap_sync(|| {
			let state: ClientStateInternal = deserialize(message.as_ref())?;
			let token = ClientState::from_deserialized(&calling_client, state).token();
			Ok(serialize(token)?.into())
		})
	}

	pub fn set_transform(&self, transform: Mat4) {
		let spatial = self.node.spatial.get().unwrap();
		spatial.set_spatial_parent(None).unwrap();
		spatial.set_local_transform(transform);
	}
	pub fn save_state(&self) -> impl Future<Output = Result<ClientStateInternal>> {
		let future = self
			.node
			.execute_remote_method("save_state", Message::default());
		async move { Ok(deserialize(&future?.await?.data)?) }
	}
}

impl Drop for Root {
	fn drop(&mut self) {
		ROOT_REGISTRY.remove(self);
	}
}

macro_rules! var_env_insert {
	($env:ident, $name:ident) => {
		$env.insert(stringify!($name).to_string(), $name.get().unwrap().clone());
	};
}
pub fn get_connection_environment_flex(
	_node: &Node,
	_calling_client: Arc<Client>,
	_message: Message,
	response: MethodResponseSender,
) {
	response.wrap_sync(move || {
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

		Ok(serialize(env)?.into())
	});
}

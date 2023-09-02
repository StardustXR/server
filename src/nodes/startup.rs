#[cfg(feature = "xwayland")]
use crate::wayland::xwayland::DISPLAY;
use crate::{
	core::{client::Client, scenegraph::MethodResponseSender},
	wayland::WAYLAND_DISPLAY,
	STARDUST_INSTANCE,
};

use super::{
	items::{ItemAcceptor, TypeInfo},
	spatial::find_spatial,
	Message, Node,
};
use color_eyre::eyre::Result;
use glam::Mat4;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use stardust_xr::schemas::flex::{deserialize, serialize};
use std::{
	fmt::Debug,
	sync::{Arc, Weak},
};

lazy_static::lazy_static! {
	pub static ref STARTUP_SETTINGS: Mutex<FxHashMap<String, StartupSettings>> = Default::default();
}

#[derive(Default, Clone)]
pub struct StartupSettings {
	pub transform: Mat4,
	pub acceptors: FxHashMap<&'static TypeInfo, Weak<ItemAcceptor>>,
}
impl StartupSettings {
	pub fn add_to(node: &Arc<Node>) {
		let _ = node
			.startup_settings
			.set(Mutex::new(StartupSettings::default()));
	}

	fn set_root_flex(node: &Node, calling_client: Arc<Client>, message: Message) -> Result<()> {
		let spatial = find_spatial(
			&calling_client,
			"Root spatial",
			deserialize(message.as_ref())?,
		)?;
		node.startup_settings.get().unwrap().lock().transform = spatial.global_transform();

		Ok(())
	}

	fn add_automatic_acceptor_flex(
		node: &Node,
		calling_client: Arc<Client>,
		message: Message,
	) -> Result<()> {
		let acceptor_node =
			calling_client.get_node("Item acceptor", deserialize(message.as_ref())?)?;
		let acceptor =
			acceptor_node.get_aspect("Item acceptor", "item acceptor", |n| &n.item_acceptor)?;
		let mut startup_settings = node.startup_settings.get().unwrap().lock();
		startup_settings
			.acceptors
			.insert(acceptor.type_info, Arc::downgrade(acceptor));

		Ok(())
	}

	fn generate_startup_token_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		_message: Message,
		response: MethodResponseSender,
	) {
		response.wrap_sync(move || {
			let id = nanoid::nanoid!();
			let data = serialize(&id)?;
			STARTUP_SETTINGS
				.lock()
				.insert(id, node.startup_settings.get().unwrap().lock().clone());
			Ok(data.into())
		});
	}
}
impl Debug for StartupSettings {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("StartupSettings")
			.field("transform", &self.transform)
			.field(
				"acceptors",
				&self
					.acceptors
					.iter()
					.map(|(k, _)| k.type_name)
					.collect::<Vec<_>>(),
			)
			.finish()
	}
}

pub fn create_interface(client: &Arc<Client>) -> Result<()> {
	let node = Node::create(client, "", "startup", false);
	node.add_local_signal("create_startup_settings", create_startup_settings_flex);
	node.add_local_method(
		"get_connection_environment",
		get_connection_environment_flex,
	);
	node.add_to_scenegraph().map(|_| ())
}

pub fn create_startup_settings_flex(
	_node: &Node,
	calling_client: Arc<Client>,
	message: Message,
) -> Result<()> {
	let node = Node::create(
		&calling_client,
		"/startup/settings",
		deserialize(message.as_ref())?,
		true,
	)
	.add_to_scenegraph()?;
	StartupSettings::add_to(&node);

	node.add_local_signal("set_root", StartupSettings::set_root_flex);
	node.add_local_signal(
		"add_automatic_acceptor",
		StartupSettings::add_automatic_acceptor_flex,
	);
	node.add_local_method(
		"generate_startup_token",
		StartupSettings::generate_startup_token_flex,
	);

	Ok(())
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
			var_env_insert!(env, DISPLAY);
			env.insert("GDK_BACKEND".to_string(), "wayland".to_string());
			env.insert("QT_QPA_PLATFORM".to_string(), "wayland".to_string());
			env.insert("MOZ_ENABLE_WAYLAND".to_string(), "1".to_string());
			env.insert("CLUTTER_BACKEND".to_string(), "wayland".to_string());
			env.insert("SDL_VIDEODRIVER".to_string(), "wayland".to_string());
		}

		Ok(serialize(env)?.into())
	});
}

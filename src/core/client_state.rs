use super::client::{ConnectedClient, get_env};
use crate::{core::Id, nodes::spatial::SpatialMut};
use dashmap::DashMap;
use glam::Mat4;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use stardust_xr_protocol::protocol::{client::ClientState, spatial::SpatialRef};
use std::{
	collections::HashMap,
	path::{Path, PathBuf},
	process::Command,
	sync::{Arc, LazyLock},
};

pub static CLIENT_STATES: LazyLock<DashMap<String, Arc<ClientStateParsed>>> =
	LazyLock::new(Default::default);

#[derive(Debug, Serialize, Deserialize)]
pub struct LaunchInfo {
	pub cmdline: Vec<String>,
	pub cwd: PathBuf,
	pub env: FxHashMap<String, String>,
}
impl LaunchInfo {
	fn from_client(client: &ConnectedClient) -> Option<Self> {
		Some(LaunchInfo {
			cmdline: client.get_cmdline()?,
			cwd: client.get_cwd()?,
			env: get_env(client.pid?).ok()?,
		})
	}
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClientStateParsed {
	pub launch_info: Option<LaunchInfo>,
	#[serde(skip)]
	pub data: Option<Vec<u8>>,
	pub root: Mat4,
	pub spatial_anchors: FxHashMap<String, Mat4>,
}
impl ClientStateParsed {
	pub fn from_deserialized(client: &ConnectedClient, state: ClientState) -> Self {
		ClientStateParsed {
			launch_info: LaunchInfo::from_client(client),
			data: state.data,
			root: Self::spatial_transform(client, state.root).unwrap_or_default(),
			spatial_anchors: state
				.spatial_anchors
				.into_iter()
				.filter_map(|(k, v)| Some((k, Self::spatial_transform(client, v)?)))
				.collect(),
		}
	}
	fn spatial_transform(client: &ConnectedClient, id: Id) -> Option<Mat4> {
		let node = client.scenegraph.get_node(id)?;
		let spatial = node.get_aspect::<SpatialMut>().ok()?;
		Some(spatial.global_transform())
	}

	pub fn token(self) -> String {
		let token = nanoid::nanoid!();
		CLIENT_STATES.insert(token.clone(), Arc::new(self));
		token
	}
	pub fn from_file(file: &Path) -> Option<Self> {
		let file_string = std::fs::read_to_string(file).ok()?;
		let mut client_state: Self = toml::from_str(&file_string).ok()?;
		client_state.data = std::fs::read(file.with_extension("bin")).ok();
		Some(client_state)
	}
	pub fn to_file(&self, directory: &Path) {
		let app_name = self
			.launch_info
			.as_ref()
			.map(|l| l.cmdline.first().unwrap().split('/').next_back().unwrap())
			.unwrap_or("unknown");
		let state_file_prefix = directory.join(format!("{app_name}-{}", nanoid::nanoid!()));
		let state_metadata_path = state_file_prefix.with_extension("toml");
		let state_data_path = state_file_prefix.with_extension("bin");

		std::fs::write(state_metadata_path, toml::to_string(&self).unwrap()).unwrap();
		if let Some(data) = self.data.as_deref() {
			std::fs::write(state_data_path, data).unwrap();
		}
	}

	pub fn apply_to(&self, client: &Arc<ConnectedClient>) -> ClientState {
		let root_spatial = SpatialMut::new(None, self.root);
		let root = root_spatial.get_ref();
		let mut spatial_anchors = HashMap::new();
		for (k, v) in self.spatial_anchors.iter() {
			let spatial = SpatialMut::new(Some(root.clone()), *v);
			spatial_anchors.insert(
				k.clone(),
				SpatialRef::from_handler(spatial.get_ref()),
			);
		}
		ClientState {
			data: self.data.clone(),
			root: SpatialRef::from_handler(root),
			spatial_anchors,
		}
	}
	pub fn launch_command(self) -> Option<Command> {
		let launch_info = self.launch_info.as_ref()?;
		let mut cmdline = launch_info.cmdline.iter();
		let mut command = Command::new(cmdline.next()?);
		command.args(cmdline);
		command.current_dir(&launch_info.cwd);
		command.envs(launch_info.env.iter());
		command.env("STARDUST_STARTUP_TOKEN", self.token());
		Some(command)
	}
}
impl Default for ClientStateParsed {
	fn default() -> Self {
		Self {
			launch_info: None,
			data: None,
			root: Mat4::IDENTITY,
			spatial_anchors: Default::default(),
		}
	}
}

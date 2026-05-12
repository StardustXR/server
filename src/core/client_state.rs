use super::client::ConnectedClient;
use crate::nodes::{ProxyExt, spatial::SpatialObject};
use dashmap::DashMap;
use glam::Mat4;
use serde::{Deserialize, Serialize};
use stardust_xr_protocol::spatial::SpatialRef;
use std::{
	path::Path,
	process::Command,
	sync::{Arc, LazyLock},
};

pub static CLIENT_STATES: LazyLock<DashMap<String, Arc<ClientStateParsed>>> =
	LazyLock::new(Default::default);

// #[derive(Debug, Serialize, Deserialize)]
// pub struct LaunchInfo {
// 	pub cmdline: Vec<String>,
// 	pub cwd: PathBuf,
// 	pub env: FxHashMap<String, String>,
// }
// impl LaunchInfo {
// 	fn from_client(client: &ConnectedClient) -> Option<Self> {
// 		Some(LaunchInfo {
// 			cmdline: client.get_cmdline()?,
// 			cwd: client.get_cwd()?,
// 			env: get_env(client.pid).ok()?,
// 		})
// 	}
// }

#[derive(Debug, Serialize, Deserialize)]
pub struct ClientStateParsed {
	// pub launch_info: Option<LaunchInfo>,
	// #[serde(skip)]
	// pub data: Option<Vec<u8>>,
	pub root: Mat4,
	// pub spatial_anchors: FxHashMap<String, Mat4>,
}
impl ClientStateParsed {
	pub fn from_deserialized(_client: &ConnectedClient, root: &SpatialRef) -> Self {
		ClientStateParsed {
			// launch_info: LaunchInfo::from_client(client),
			// data: state.data,
			root: root
				.owned()
				.map(|v| v.global_transform())
				.unwrap_or_default(),
			// spatial_anchors: state
			// 	.spatial_anchors
			// 	.into_iter()
			// 	.filter_map(|(k, v)| Some((k, v.owned()?.global_transform())))
			// 	.collect(),
		}
	}

	pub fn token(self) -> String {
		let token = nanoid::nanoid!();
		CLIENT_STATES.insert(token.clone(), Arc::new(self));
		token
	}
	pub fn from_file(file: &Path) -> Option<Self> {
		let file_string = std::fs::read_to_string(file).ok()?;
		let /* mut */ client_state: Self = toml::from_str(&file_string).ok()?;
		// client_state.data = std::fs::read(file.with_extension("bin")).ok();
		Some(client_state)
	}
	pub fn to_file(&self, directory: &Path) {
		// let app_name = self
		// 	.launch_info
		// 	.as_ref()
		// 	.map(|l| l.cmdline.first().unwrap().split('/').next_back().unwrap())
		// 	.unwrap_or("unknown");
		let app_name = "unknown";
		let state_file_prefix = directory.join(format!("{app_name}-{}", nanoid::nanoid!()));
		let state_metadata_path = state_file_prefix.with_extension("toml");
		// let state_data_path = state_file_prefix.with_extension("bin");

		std::fs::write(state_metadata_path, toml::to_string(&self).unwrap()).unwrap();
		// if let Some(data) = self.data.as_deref() {
		// 	std::fs::write(state_data_path, data).unwrap();
		// }
	}

	pub fn apply(&self) -> SpatialRef {
		let root_spatial = SpatialObject::new(None, self.root);
		let root = root_spatial.get_ref();
		// ClientState {
		// 	data: self.data.clone(),
		// 	root: SpatialRef::from_handler(root),
		// 	spatial_anchors: self
		// 		.spatial_anchors
		// 		.iter()
		// 		.map(|(k, v)| {
		// 			let spatial = SpatialObject::new(Some(root), *v);
		// 			(k.clone(), SpatialRef::from_handler(spatial.get_ref()))
		// 		})
		// 		.collect(),
		// }
		SpatialRef::from_handler(root)
	}
	pub fn launch_command(self) -> Option<Command> {
		None
		// let launch_info = self.launch_info.as_ref()?;
		// let mut cmdline = launch_info.cmdline.iter();
		// let mut command = Command::new(cmdline.next()?);
		// command.args(cmdline);
		// command.current_dir(&launch_info.cwd);
		// command.envs(launch_info.env.iter());
		// command.env("STARDUST_STARTUP_TOKEN", self.token());
		// Some(command)
	}
}
impl Default for ClientStateParsed {
	fn default() -> Self {
		Self {
			// launch_info: None,
			// data: None,
			root: Mat4::IDENTITY,
			// spatial_anchors: Default::default(),
		}
	}
}

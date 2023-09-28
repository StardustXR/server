use super::client::Client;
use crate::nodes::{spatial::Spatial, Node};
use glam::Mat4;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};

lazy_static::lazy_static! {
	pub static ref CLIENT_STATES: Mutex<FxHashMap<String, Arc<ClientState>>> = Default::default();
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClientState {
	pub cmdline: Option<Vec<String>>,
	pub cwd: Option<PathBuf>,
	pub data: Option<Vec<u8>>,
	pub root: Mat4,
	pub spatial_anchors: FxHashMap<String, Mat4>,
}
impl ClientState {
	pub fn from_deserialized(client: &Client, state: ClientStateInternal) -> Self {
		ClientState {
			cmdline: client.get_cmdline(),
			cwd: client.get_cwd(),
			data: state.data,
			root: Self::spatial_transform(client, &state.root.unwrap_or_default())
				.unwrap_or_default(),
			spatial_anchors: state
				.spatial_anchors
				.into_iter()
				.filter_map(|(k, v)| Some((k, Self::spatial_transform(client, &v)?)))
				.collect(),
		}
	}
	fn spatial_transform(client: &Client, path: &str) -> Option<Mat4> {
		let node = client.scenegraph.get_node(path)?;
		let spatial = node.spatial.get()?;
		Some(spatial.global_transform())
	}

	pub fn token(self) -> String {
		let token = nanoid::nanoid!();
		CLIENT_STATES.lock().insert(token.clone(), Arc::new(self));
		token
	}
	pub fn to_file(self) {
		let state_dir = directories::ProjectDirs::from("", "", "stardust")
			.unwrap()
			.state_dir()
			.unwrap();
		// std::fs::File::create(state_dir.join(""))
	}
	pub fn apply_to(&self, client: &Arc<Client>) -> ClientStateInternal {
		if let Some(root) = client.root.get() {
			root.set_transform(self.root)
		}
		ClientStateInternal {
			data: self.data.clone(),
			root: Some("/".to_string()),
			spatial_anchors: self
				.spatial_anchors
				.iter()
				.map(|(k, v)| {
					(k.clone(), {
						let node = Node::create(client, "/spatial/anchor", k, true)
							.add_to_scenegraph()
							.unwrap();
						Spatial::add_to(&node, None, *v, false).unwrap();
						k.clone()
					})
				})
				.collect(),
		}
	}
}
impl Default for ClientState {
	fn default() -> Self {
		Self {
			cmdline: None,
			cwd: None,
			data: None,
			root: Mat4::IDENTITY,
			spatial_anchors: Default::default(),
		}
	}
}

#[derive(Default, Serialize, Deserialize)]
pub struct ClientStateInternal {
	data: Option<Vec<u8>>,
	root: Option<String>,
	spatial_anchors: FxHashMap<String, String>,
}

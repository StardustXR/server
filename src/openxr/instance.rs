use super::{action_set::ActionSet, Object};
use crate::{core::client::Client, nodes::Node};
use color_eyre::eyre::{bail, eyre, Result};
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use serde::Deserialize;
use stardust_xr::schemas::flex::deserialize;
use std::sync::{Arc, Weak};

#[derive(Debug, Deserialize)]
struct InstanceInfo {
	_app_info: ApplicationInfo,
	_extension_names: Vec<String>,
}
#[derive(Debug, Deserialize)]
struct ApplicationInfo {
	_app_name: String,
	_app_version: u32,
	_engine_name: String,
	_engine_version: u32,
	_api_version: u64,
}

#[derive(Debug)]
pub struct Instance {
	_info: InstanceInfo,
	pub action_sets: Mutex<FxHashMap<String, Weak<ActionSet>>>,
}
impl Instance {
	pub fn setup_instance_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		data: &[u8],
	) -> Result<()> {
		let Object::Instance(instance) = node.get_aspect("OpenXR interface", "Instance", |n| &n.openxr_object)? else {
			bail!("Object not an instance")
		};
		let instance_info = Instance {
			_info: deserialize(data)?,
			action_sets: Mutex::new(FxHashMap::default()),
		};
		dbg!(&instance_info);
		instance
			.set(Arc::new(instance_info))
			.map_err(|_| eyre!("Instance already set up"))
	}
}

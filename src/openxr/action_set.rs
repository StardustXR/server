use super::{action::Action, Object};
use crate::{core::client::Client, nodes::Node};
use color_eyre::eyre::{bail, Result};
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use serde::Deserialize;
use stardust_xr::schemas::flex::deserialize;
use std::sync::{Arc, Weak};

#[derive(Debug)]
pub struct ActionSet {
	// _info: InstanceInfo,
	_localized_name: String,
	_priority: u32,
	pub actions: Mutex<FxHashMap<String, Weak<Action>>>,
}
impl ActionSet {
	pub fn create_action_set_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		data: &[u8],
	) -> Result<()> {
		let Object::Instance(instance) = node.get_aspect("OpenXR interface", "Instance", |n| &n.openxr_object)? else {
			bail!("Object not an instance")
		};
		let Some(instance) = instance.get() else { bail!("Instance not initialized") };

		#[derive(Deserialize)]
		struct CreateActionSetInfo {
			name: String,
			localized_name: String,
			priority: u32,
		}
		let info: CreateActionSetInfo = deserialize(data)?;

		let node = Node::create(
			&node.get_client().unwrap(),
			"/openxr/action_set",
			&info.name,
			true,
		)
		.add_to_scenegraph();
		node.add_local_signal("create_action", Action::create_action_flex);

		let action_set = Arc::new(ActionSet {
			_localized_name: info.localized_name,
			_priority: info.priority,
			actions: Mutex::new(FxHashMap::default()),
		});
		instance
			.action_sets
			.lock()
			.insert(info.name, Arc::downgrade(&action_set));
		node.openxr_object
			.set(Object::ActionSet(action_set))
			.unwrap();

		Ok(())
	}
}

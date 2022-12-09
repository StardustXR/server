use super::Object;
use crate::{core::client::Client, nodes::Node};
use color_eyre::eyre::{bail, Result};
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use serde::Deserialize;
use stardust_xr::schemas::flex::deserialize;
use std::sync::Arc;

#[derive(Debug)]
pub struct Action {
	// _info: InstanceInfo,
	_localized_name: String,
	suggested_bindings: Mutex<FxHashMap<String, String>>,
}
impl Action {
	pub fn create_action_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		data: &[u8],
	) -> Result<()> {
		let Object::ActionSet(action_set) = node.get_aspect("OpenXR interface", "Instance", |n| &n.openxr_object)? else {
			bail!("Object not an instance")
		};

		#[derive(Debug, Deserialize)]
		struct CreateActionInfo {
			name: String,
			localized_name: String,
		}
		let info: CreateActionInfo = dbg!(deserialize(data)?);

		let node = Node::create(
			&node.get_client().unwrap(),
			node.get_path(),
			&info.name,
			true,
		)
		.add_to_scenegraph();
		node.add_local_signal("suggest_binding", Self::suggest_binding_flex);

		let action = Arc::new(Action {
			_localized_name: info.localized_name,
			suggested_bindings: Mutex::new(FxHashMap::default()),
		});
		action_set
			.actions
			.lock()
			.insert(info.name, Arc::downgrade(&action));
		node.openxr_object.set(Object::Action(action)).unwrap();

		Ok(())
	}
	pub fn suggest_binding_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		data: &[u8],
	) -> Result<()> {
		let Object::Action(action) = node.get_aspect("OpenXR interface", "Action", |n| &n.openxr_object)? else {
			bail!("Object not an action")
		};

		#[derive(Debug, Deserialize)]
		struct SuggestBindingArgs {
			interaction_profile: String,
			binding: String,
		}
		let args: SuggestBindingArgs = dbg!(deserialize(data)?);
		action
			.suggested_bindings
			.lock()
			.insert(args.interaction_profile, args.binding);

		Ok(())
	}
}

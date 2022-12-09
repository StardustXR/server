mod action;
mod action_set;
mod instance;
mod session;
mod system;

use self::{
	action::Action, action_set::ActionSet, instance::Instance, session::Session, system::System,
};
use crate::{core::client::Client, nodes::Node};
use once_cell::sync::OnceCell;
use std::sync::Arc;

#[derive(Debug)]
pub enum Object {
	Instance(OnceCell<Arc<Instance>>),
	System(System),
	Session(Session),
	ActionSet(Arc<ActionSet>),
	Action(Arc<Action>),
}

pub fn create_interface(client: &Arc<Client>) {
	let node = Node::create(client, "", "openxr", false);
	node.add_local_signal("setup_instance", Instance::setup_instance_flex);
	node.add_local_method("get_system", System::get_system_flex);
	node.add_local_signal("create_action_set", ActionSet::create_action_set_flex);

	node.openxr_object
		.set(Object::Instance(OnceCell::new()))
		.unwrap();

	node.add_to_scenegraph();
}

use std::sync::Arc;

use color_eyre::eyre::{bail, Result};
use stardust_xr::schemas::flex::deserialize;

use super::Object;
use crate::{core::client::Client, nodes::Node};

#[derive(Debug)]
pub struct Session {
	// _info: InstanceInfo,
}
impl Session {
	pub fn create_session_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		data: &[u8],
	) -> Result<()> {
		let Object::System(_system) = node.get_aspect("OpenXR interface", "Instance", |n| &n.openxr_object)? else {
			bail!("Object not a system")
		};
		let node = Node::create(
			&node.get_client().unwrap(),
			node.get_path(),
			deserialize(data)?,
			true,
		)
		.add_to_scenegraph();
		let session = Session {};
		node.openxr_object.set(Object::Session(session)).unwrap();

		Ok(())
	}
}

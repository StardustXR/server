use super::{session::Session, Object};
use crate::{core::client::Client, nodes::Node, SK_INFO};
use color_eyre::eyre::{bail, eyre, Result};
use serde::Serialize;
use stardust_xr::schemas::flex::{deserialize, serialize};
use std::sync::Arc;

#[derive(Debug)]
pub enum System {
	Handheld,
	HeadMounted,
}
impl System {
	pub fn from_raw(raw: u32) -> Option<Self> {
		match raw {
			1 => Some(System::Handheld),
			2 => Some(System::HeadMounted),
			_ => None,
		}
	}

	pub fn get_system_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		data: &[u8],
	) -> Result<Vec<u8>> {
		// let Object::Instance(instance) = node.get_aspect("OpenXR interface", "Instance", |n| &n.openxr_object)? else {
		// 	bail!("Object not an instance")
		// };
		let system_type: u32 = deserialize(data)?;
		let system = System::from_raw(system_type).ok_or_else(|| eyre!("No system exists!"))?;
		let node = Node::create(
			&node.get_client().unwrap(),
			node.get_path(),
			&format!("system{}", system_type),
			true,
		)
		.add_to_scenegraph();
		node.add_local_method("views", System::views_flex);
		node.add_local_signal("create_session", Session::create_session_flex);
		node.openxr_object.set(Object::System(system)).unwrap();

		Ok(serialize(system_type)?)
	}

	fn views_flex(_node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<Vec<u8>> {
		let view_configuration_type: u64 = deserialize(data)?;
		let view_count: u32 = match view_configuration_type {
			1 => 1,
			2 => 2,
			1000037000 => 4,
			1000054000 => 1,
			_ => bail!("Invalid view config type"),
		};

		#[derive(Debug, Serialize)]
		struct View {
			recommended_image_rect_width: u32,
			max_image_rect_width: u32,
			recommended_image_rect_height: u32,
			max_image_rect_height: u32,
		}
		let sk_info = SK_INFO.get().unwrap();

		Ok(serialize(
			(0..view_count)
				.map(|_| View {
					recommended_image_rect_width: sk_info.display_width,
					max_image_rect_width: sk_info.display_width,
					recommended_image_rect_height: sk_info.display_height,
					max_image_rect_height: sk_info.display_height,
				})
				.collect::<Vec<_>>(),
		)?)
	}
}

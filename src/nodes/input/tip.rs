use super::{DistanceLink, InputSpecialization};
use crate::core::client::Client;
use crate::nodes::fields::Field;
use crate::nodes::input::{InputMethod, InputType};
use crate::nodes::spatial::{find_spatial_parent, parse_transform, Spatial, Transform};
use crate::nodes::{Message, Node};
use color_eyre::eyre::Result;
use glam::{vec3a, Mat4};
use serde::Deserialize;
use stardust_xr::schemas::flat::{InputDataType, Tip as FlatTip};
use stardust_xr::schemas::flex::deserialize;
use stardust_xr::values::Datamap;

use std::sync::Arc;

#[derive(Default)]
pub struct Tip {
	pub radius: f32,
}
impl Tip {
	fn set_radius(node: Arc<Node>, _calling_client: Arc<Client>, message: Message) -> Result<()> {
		if let InputType::Tip(tip) = &mut *node.input_method.get().unwrap().specialization.lock() {
			tip.radius = deserialize(message.as_ref())?;
		}
		Ok(())
	}
}
impl InputSpecialization for Tip {
	fn compare_distance(&self, space: &Arc<Spatial>, field: &Field) -> f32 {
		field.distance(space, vec3a(0.0, 0.0, 0.0)).abs()
	}
	fn true_distance(&self, space: &Arc<Spatial>, field: &Field) -> f32 {
		field.distance(space, vec3a(0.0, 0.0, 0.0))
	}
	fn serialize(
		&self,
		_distance_link: &DistanceLink,
		local_to_handler_matrix: Mat4,
	) -> InputDataType {
		let (_, orientation, origin) = local_to_handler_matrix.to_scale_rotation_translation();
		InputDataType::Tip(FlatTip {
			origin: origin.into(),
			orientation: orientation.into(),
			radius: self.radius,
		})
	}
}

pub fn create_tip_flex(
	_node: Arc<Node>,
	calling_client: Arc<Client>,
	message: Message,
) -> Result<()> {
	#[derive(Deserialize)]
	struct CreateTipInfo<'a> {
		name: &'a str,
		parent_path: &'a str,
		transform: Transform,
		radius: f32,
		datamap: Option<Vec<u8>>,
	}
	let info: CreateTipInfo = deserialize(message.as_ref())?;
	let node = Node::create_parent_name(&calling_client, "/input/method/tip", info.name, true);
	let parent = find_spatial_parent(&calling_client, info.parent_path)?;
	let transform = parse_transform(info.transform, true, true, false);

	let node = node.add_to_scenegraph()?;
	Spatial::add_to(&node, Some(parent), transform, false)?;
	InputMethod::add_to(
		&node,
		InputType::Tip(Tip {
			radius: info.radius,
		}),
		info.datamap
			.and_then(|datamap| Datamap::from_raw(datamap).ok()),
	)?;
	node.add_local_signal("set_radius", Tip::set_radius);
	Ok(())
}

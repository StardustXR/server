use super::{DistanceLink, InputSpecialization};
use crate::core::client::Client;
use crate::nodes::fields::Field;
use crate::nodes::input::{InputMethod, InputType};
use crate::nodes::spatial::{get_spatial_parent_flex, parse_transform, Spatial};
use crate::nodes::Node;
use anyhow::Result;
use glam::{vec3a, Mat4};
use serde::Deserialize;
use stardust_xr::schemas::flat::{Datamap, InputDataType, Tip as FlatTip};
use stardust_xr::schemas::flex::deserialize;
use stardust_xr::values::Transform;
use std::sync::Arc;

#[derive(Default)]
pub struct Tip {
	pub radius: f32,
}
impl Tip {
	fn set_radius(node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		if let InputType::Tip(tip) = &mut *node.input_method.get().unwrap().specialization.lock() {
			tip.radius = deserialize(data)?;
		}
		Ok(())
	}
}
impl InputSpecialization for Tip {
	fn distance(&self, space: &Arc<Spatial>, field: &Field) -> f32 {
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

pub fn create_tip_flex(_node: &Node, calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
	#[derive(Deserialize)]
	struct CreateTipInfo<'a> {
		name: &'a str,
		parent_path: &'a str,
		transform: Transform,
		radius: f32,
		datamap: Option<Vec<u8>>,
	}
	let info: CreateTipInfo = deserialize(data)?;
	let node = Node::create(&calling_client, "/input/method/tip", info.name, true);
	let parent = get_spatial_parent_flex(&calling_client, info.parent_path)?;
	let transform = parse_transform(info.transform, true, true, false)?;

	let node = node.add_to_scenegraph();
	Spatial::add_to(&node, Some(parent), transform)?;
	InputMethod::add_to(
		&node,
		InputType::Tip(Tip {
			radius: info.radius,
		}),
		info.datamap.and_then(|datamap| Datamap::new(datamap).ok()),
	)?;
	node.add_local_signal("setRadius", Tip::set_radius);
	Ok(())
}

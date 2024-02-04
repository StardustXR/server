use super::{DistanceLink, InputSpecialization};
use crate::core::client::Client;
use crate::nodes::fields::{Field, Ray, RayMarchResult};
use crate::nodes::input::{InputMethod, InputType};
use crate::nodes::spatial::{find_spatial_parent, parse_transform, Spatial, Transform};
use crate::nodes::{Message, Node};
use glam::{vec3, Mat4};
use serde::Deserialize;
use stardust_xr::schemas::flat::{InputDataType, Pointer as FlatPointer};
use stardust_xr::schemas::flex::deserialize;
use stardust_xr::values::Datamap;

use std::sync::Arc;

#[derive(Default)]
pub struct Pointer;
// impl Default for Pointer {
// 	fn default() -> Self {
// 		Pointer {
// 			grab: Default::default(),
// 			select: Default::default(),
// 		}
// 	}
// }
impl Pointer {
	fn ray_march(&self, space: &Arc<Spatial>, field: &Field) -> RayMarchResult {
		field.ray_march(Ray {
			origin: vec3(0.0, 0.0, 0.0),
			direction: vec3(0.0, 0.0, -1.0),
			space: space.clone(),
		})
	}
}

impl InputSpecialization for Pointer {
	fn compare_distance(&self, space: &Arc<Spatial>, field: &Field) -> f32 {
		let ray_info = self.ray_march(space, field);
		if ray_info.min_distance > 0.0 {
			ray_info.deepest_point_distance + 1000.0
		} else {
			ray_info
				.deepest_point_distance
				.hypot(0.001 / ray_info.min_distance)
		}
	}
	fn true_distance(&self, space: &Arc<Spatial>, field: &Field) -> f32 {
		let ray_info = self.ray_march(space, field);
		ray_info.min_distance
	}
	fn serialize(
		&self,
		distance_link: &DistanceLink,
		local_to_handler_matrix: Mat4,
	) -> InputDataType {
		let (_, orientation, origin) = local_to_handler_matrix.to_scale_rotation_translation();
		let direction = local_to_handler_matrix.transform_vector3(vec3(0.0, 0.0, -1.0));
		let ray_march = self.ray_march(&distance_link.method.spatial, &distance_link.handler.field);
		let deepest_point = (direction * ray_march.deepest_point_distance) + origin;

		InputDataType::Pointer(FlatPointer {
			origin: origin.into(),
			orientation: orientation.into(),
			deepest_point: deepest_point.into(),
		})
	}
}

pub fn create_pointer_flex(
	_node: Arc<Node>,
	calling_client: Arc<Client>,
	message: Message,
) -> color_eyre::eyre::Result<()> {
	#[derive(Deserialize)]
	struct CreatePointerInfo<'a> {
		name: &'a str,
		parent_path: &'a str,
		transform: Transform,
		datamap: Option<Vec<u8>>,
	}
	let info: CreatePointerInfo = deserialize(message.as_ref())?;
	let node = Node::create_parent_name(&calling_client, "/input/method/pointer", info.name, true);
	let parent = find_spatial_parent(&calling_client, info.parent_path)?;
	let transform = parse_transform(info.transform, true, true, false);

	let node = node.add_to_scenegraph()?;
	Spatial::add_to(&node, Some(parent), transform, false)?;
	InputMethod::add_to(
		&node,
		InputType::Pointer(Pointer),
		info.datamap
			.and_then(|datamap| Datamap::from_raw(datamap).ok()),
	)?;
	Ok(())
}

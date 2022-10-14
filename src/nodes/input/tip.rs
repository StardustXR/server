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
}

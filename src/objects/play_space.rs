use std::sync::Arc;

use color_eyre::eyre::Result;
use glam::Mat4;
use mint::Vector2;
use nanoid::nanoid;
use serde::{Deserialize, Serialize};
use stereokit::StereoKitMultiThread;

use crate::{
	core::client::INTERNAL_CLIENT,
	nodes::{
		data::{Mask, PulseReceiver},
		fields::{r#box::BoxField, Field},
		spatial::Spatial,
		Node,
	},
};

#[derive(Debug, Deserialize, Serialize)]
struct PlaySpaceMap {
	play_space: (),
	size: Vector2<f32>,
}
impl Default for PlaySpaceMap {
	fn default() -> Self {
		Self {
			play_space: (),
			size: [0.0; 2].into(),
		}
	}
}

pub struct PlaySpace {
	_node: Arc<Node>,
	spatial: Arc<Spatial>,
	field: Arc<Field>,
	_pulse_rx: Arc<PulseReceiver>,
}
impl PlaySpace {
	pub fn new() -> Result<Self> {
		let node = Node::create(&INTERNAL_CLIENT, "", &nanoid!(), false).add_to_scenegraph()?;
		let spatial = Spatial::add_to(&node, None, Mat4::IDENTITY, false)?;
		let field = BoxField::add_to(&node, [0.0; 3].into())?;

		let pulse_rx =
			PulseReceiver::add_to(&node, field.clone(), Mask::from_struct::<PlaySpaceMap>())?;

		Ok(PlaySpace {
			_node: node,
			spatial,
			field,
			_pulse_rx: pulse_rx,
		})
	}
	pub fn update(&self, sk: &impl StereoKitMultiThread) {
		let pose = sk.world_get_bounds_pose();
		self.spatial
			.set_local_transform(Mat4::from_rotation_translation(
				pose.orientation,
				pose.position,
			));
		let Field::Box(box_field) = self.field.as_ref() else {return};
		box_field.set_size(
			[
				sk.world_get_bounds_size().x,
				0.0,
				sk.world_get_bounds_size().y,
			]
			.into(),
		);
	}
}

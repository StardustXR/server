use std::sync::Arc;

use color_eyre::eyre::Result;
use glam::Mat4;
use mint::Vector2;
use serde::{Deserialize, Serialize};
use stardust_xr::values::Datamap;
use stereokit_rust::system::World;

use crate::{
	core::client::INTERNAL_CLIENT,
	nodes::{
		data::PulseReceiver,
		fields::{Field, Shape},
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
		let node = Node::generate(&INTERNAL_CLIENT, false).add_to_scenegraph()?;
		let spatial = Spatial::add_to(&node, None, Mat4::IDENTITY, false);
		Field::add_to(&node, Shape::Box([0.0; 3].into()))?;
		let field = node.get_aspect::<Field>()?.clone();

		let pulse_rx = PulseReceiver::add_to(
			&node,
			field.clone(),
			Datamap::from_typed(PlaySpaceMap::default())?,
		)?;

		Ok(PlaySpace {
			_node: node,
			spatial,
			field,
			_pulse_rx: pulse_rx,
		})
	}
	pub fn update(&self) {
		let pose = World::get_bounds_pose();
		self.spatial
			.set_local_transform(Mat4::from_rotation_translation(
				pose.orientation.into(),
				pose.position.into(),
			));
		*self.field.shape.lock() =
			Shape::Box([World::get_bounds_size().x, 0.0, World::get_bounds_size().y].into());
	}
}

use crate::{
	core::client::INTERNAL_CLIENT,
	nodes::{
		input::{tip::Tip, InputMethod, InputType},
		spatial::Spatial,
		Node,
	},
};
use color_eyre::eyre::Result;
use glam::Mat4;
use nanoid::nanoid;
use stardust_xr::{
	schemas::{flat::Datamap, flex::flexbuffers},
	values::Transform,
};
use std::sync::Arc;
use stereokit::{ButtonState, Handed, StereoKitMultiThread};
use tracing::instrument;

pub struct SkController {
	_node: Arc<Node>,
	input: Arc<InputMethod>,
	handed: Handed,
}
impl SkController {
	pub fn new(handed: Handed) -> Result<Self> {
		let _node = Node::create(&INTERNAL_CLIENT, "", &nanoid!(), false).add_to_scenegraph()?;
		Spatial::add_to(&_node, None, Mat4::IDENTITY, false)?;
		let tip = InputType::Tip(Tip::default());
		let input = InputMethod::add_to(&_node, tip, None)?;
		Ok(SkController {
			_node,
			input,
			handed,
		})
	}
	#[instrument(level = "debug", name = "Update StereoKit Tip Input Method", skip_all)]
	pub fn update(&mut self, sk: &impl StereoKitMultiThread) {
		let controller = sk.input_controller(self.handed);
		*self.input.enabled.lock() = controller.tracked.contains(ButtonState::ACTIVE);
		if *self.input.enabled.lock() {
			self.input.spatial.set_local_transform_components(
				None,
				Transform::from_position_rotation(
					controller.pose.position,
					controller.pose.orientation,
				),
			);
		}
		let mut fbb = flexbuffers::Builder::default();
		let mut map = fbb.start_map();
		map.push("select", controller.trigger);
		map.push("grab", controller.grip);
		map.end_map();
		*self.input.datamap.lock() = Datamap::new(fbb.take_buffer()).ok();
	}
}

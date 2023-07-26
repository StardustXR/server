use crate::{
	core::{client::INTERNAL_CLIENT, typed_datamap::TypedDatamap},
	nodes::{
		input::{tip::Tip, InputMethod, InputType},
		spatial::Spatial,
		Node,
	},
};
use color_eyre::eyre::Result;
use glam::{Mat4, Vec2};
use nanoid::nanoid;
use serde::{Deserialize, Serialize};
use stardust_xr::values::Transform;
use std::sync::Arc;
use stereokit::{
	ButtonState, Color128, Handed, Model, RenderLayer, StereoKitDraw, StereoKitMultiThread,
};
use tracing::instrument;

#[derive(Default, Deserialize, Serialize)]
struct ControllerDatamap {
	select: f32,
	grab: f32,
	scroll: Vec2,
}

pub struct SkController {
	_node: Arc<Node>,
	input: Arc<InputMethod>,
	model: Model,
	handed: Handed,
	datamap: TypedDatamap<ControllerDatamap>,
}
impl SkController {
	pub fn new(sk: &impl StereoKitMultiThread, handed: Handed) -> Result<Self> {
		let _node = Node::create(&INTERNAL_CLIENT, "", &nanoid!(), false).add_to_scenegraph()?;
		Spatial::add_to(&_node, None, Mat4::IDENTITY, false)?;
		let model = sk.model_create_mem("cursor", include_bytes!("cursor.glb"), None)?;
		let tip = InputType::Tip(Tip::default());
		let input = InputMethod::add_to(&_node, tip, None)?;
		Ok(SkController {
			_node,
			input,
			handed,
			model,
			datamap: Default::default(),
		})
	}
	#[instrument(level = "debug", name = "Update StereoKit Tip Input Method", skip_all)]
	pub fn update(&mut self, sk: &impl StereoKitDraw) {
		let controller = sk.input_controller(self.handed);
		*self.input.enabled.lock() = controller.tracked.contains(ButtonState::ACTIVE);
		if *self.input.enabled.lock() {
			sk.model_draw(
				&self.model,
				Mat4::from_rotation_translation(
					controller.aim.orientation,
					controller.aim.position,
				),
				Color128::default(),
				RenderLayer::all(),
			);
			self.input.spatial.set_local_transform_components(
				None,
				Transform::from_position_rotation(
					controller.aim.position,
					controller.aim.orientation,
				),
			);
		}
		self.datamap.select = controller.trigger;
		self.datamap.grab = controller.grip;
		self.datamap.scroll = controller.stick;
		*self.input.datamap.lock() = self.datamap.to_datamap().ok();
	}
}

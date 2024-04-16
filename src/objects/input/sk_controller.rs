use crate::{
	core::client::INTERNAL_CLIENT,
	nodes::{
		input::{InputDataType, InputHandler, InputMethod, Tip, INPUT_HANDLER_REGISTRY},
		spatial::Spatial,
		Node,
	},
};
use color_eyre::eyre::Result;
use glam::{Mat4, Vec2, Vec3};
use serde::{Deserialize, Serialize};
use stardust_xr::values::Datamap;
use std::sync::Arc;
use stereokit::{
	named_colors::WHITE, ButtonState, Handed, Model, RenderLayer, StereoKitDraw,
	StereoKitMultiThread,
};

#[derive(Default, Deserialize, Serialize)]
struct ControllerDatamap {
	select: f32,
	grab: f32,
	scroll: Vec2,
}

pub struct SkController {
	_node: Arc<Node>,
	input: Arc<InputMethod>,
	handed: Handed,
	model: Model,
	capture: Option<Arc<InputHandler>>,
	datamap: ControllerDatamap,
}
impl SkController {
	pub fn new(sk: &impl StereoKitMultiThread, handed: Handed) -> Result<Self> {
		let _node = Node::create_parent_name(
			&INTERNAL_CLIENT,
			"",
			if handed == Handed::Left {
				"controller_left"
			} else {
				"controller_right"
			},
			false,
		)
		.add_to_scenegraph()?;
		Spatial::add_to(&_node, None, Mat4::IDENTITY, false);
		let model = sk.model_create_mem("cursor.glb", include_bytes!("cursor.glb"), None)?;
		let tip = InputDataType::Tip(Tip::default());
		let input = InputMethod::add_to(
			&_node,
			tip,
			Datamap::from_typed(ControllerDatamap::default())?,
		)?;
		Ok(SkController {
			_node,
			input,
			handed,
			model,
			capture: None,
			datamap: Default::default(),
		})
	}
	pub fn update(&mut self, sk: &impl StereoKitDraw) {
		let controller = sk.input_controller(self.handed);
		*self.input.enabled.lock() = controller.tracked.contains(ButtonState::ACTIVE);
		if *self.input.enabled.lock() {
			let world_transform = Mat4::from_rotation_translation(
				controller.aim.orientation,
				controller.aim.position,
			);
			sk.model_draw(
				&self.model,
				world_transform * Mat4::from_scale(Vec3::ONE * 0.02),
				WHITE,
				RenderLayer::LAYER0,
			);
			self.input.spatial.set_local_transform(world_transform);
		}
		self.datamap.select = controller.trigger;
		self.datamap.grab = controller.grip;
		self.datamap.scroll = controller.stick;
		*self.input.datamap.lock() = Datamap::from_typed(&self.datamap).unwrap();

		// remove the capture when it's removed from captures list
		if let Some(capture) = &self.capture {
			if !self.input.captures.get_valid_contents().contains(&capture) {
				self.capture.take();
			}
		}
		// add the capture that's the closest if we don't have one
		if self.capture.is_none() {
			self.capture = self
				.input
				.captures
				.get_valid_contents()
				.into_iter()
				.map(|handler| {
					(
						handler.clone(),
						handler
							.field
							.distance(&self.input.spatial, [0.0; 3].into())
							.abs(),
					)
				})
				.reduce(|(handlers_a, distance_a), (handlers_b, distance_b)| {
					if distance_a < distance_b {
						(handlers_a, distance_a)
					} else {
						(handlers_b, distance_b)
					}
				})
				.map(|(rx, _)| rx);
		}

		// make sure that if something is captured only send input to it
		if let Some(capture) = &self.capture {
			self.input.set_handler_order([capture].into_iter());
			return;
		}

		// send input to all the input handlers that are the closest to the ray as possible
		self.input.set_handler_order(
			INPUT_HANDLER_REGISTRY
				.get_valid_contents()
				.into_iter()
				// filter out all the disabled handlers
				.filter(|handler| {
					let Some(node) = handler.node.upgrade() else {
						return false;
					};
					node.enabled()
				})
				// get the unsigned distance to the handler's field (unsigned so giant fields won't always eat input)
				.map(|handler| {
					(
						vec![handler.clone()],
						handler
							.field
							.distance(&self.input.spatial, [0.0; 3].into())
							.abs(),
					)
				})
				// .inspect(|(_, result)| {
				// 	dbg!(result);
				// })
				// now collect all handlers that are same distance if they're the closest
				.reduce(|(mut handlers_a, distance_a), (handlers_b, distance_b)| {
					if (distance_a - distance_b).abs() < 0.001 {
						// distance is basically the same (within 1mm)
						handlers_a.extend(handlers_b);
						(handlers_a, distance_a)
					} else if distance_a < distance_b {
						(handlers_a, distance_a)
					} else {
						(handlers_b, distance_b)
					}
				})
				.map(|(rx, _)| rx)
				.unwrap_or_default()
				.iter(),
		);
	}
}

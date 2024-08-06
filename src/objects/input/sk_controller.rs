use super::{get_sorted_handlers, CaptureManager};
use crate::{
	core::client::INTERNAL_CLIENT,
	nodes::{
		fields::{Field, FieldTrait},
		input::{InputDataType, InputHandler, InputMethod, Tip, INPUT_HANDLER_REGISTRY},
		spatial::Spatial,
		Node, OwnedNode,
	},
	objects::{ObjectHandle, SpatialRef},
};
use color_eyre::eyre::Result;
use glam::{Mat4, Vec2, Vec3};
use serde::{Deserialize, Serialize};
use stardust_xr::values::Datamap;
use std::sync::Arc;
use stereokit_rust::{
	material::Material,
	model::Model,
	sk::MainThreadToken,
	system::{Handed, Input},
	util::Color128,
};
use zbus::Connection;

#[derive(Default, Deserialize, Serialize)]
struct ControllerDatamap {
	select: f32,
	grab: f32,
	scroll: Vec2,
}

pub struct SkController {
	object_handle: ObjectHandle<SpatialRef>,
	input: Arc<InputMethod>,
	handed: Handed,
	model: Model,
	material: Material,
	capture_manager: CaptureManager,
	datamap: ControllerDatamap,
}
impl SkController {
	pub fn new(connection: &Connection, handed: Handed) -> Result<Self> {
		let (spatial, object_handle) = SpatialRef::create(
			connection,
			&("/org/stardustxr/Controller/".to_string()
				+ match handed {
					Handed::Left => "left",
					_ => "right",
				}),
		);
		let model = Model::copy(Model::from_memory(
			"cursor.glb",
			include_bytes!("cursor.glb"),
			None,
		)?);
		let model_nodes = model.get_nodes();
		let mut model_node = model_nodes.visuals().next().unwrap();
		let material = Material::copy(model_node.get_material().unwrap());
		model_node.material(&material);
		let tip = InputDataType::Tip(Tip::default());
		let input = InputMethod::add_to(
			&spatial.node().unwrap(),
			tip,
			Datamap::from_typed(ControllerDatamap::default())?,
		)?;
		Ok(SkController {
			object_handle,
			input,
			handed,
			model,
			material,
			capture_manager: CaptureManager::default(),
			datamap: Default::default(),
		})
	}
	pub fn update(&mut self, token: &MainThreadToken) {
		let controller = Input::controller(self.handed);
		let input_node = self.input.spatial.node().unwrap();
		input_node.set_enabled(controller.tracked.is_active());
		if input_node.enabled() {
			let world_transform = Mat4::from_rotation_translation(
				controller.aim.orientation.into(),
				controller.aim.position.into(),
			);
			self.material
				.color_tint(if self.capture_manager.capture.is_none() {
					Color128::new_rgb(1.0, 1.0, 1.0)
				} else {
					Color128::new_rgb(0.0, 1.0, 0.75)
				});
			self.model.draw(
				token,
				world_transform * Mat4::from_scale(Vec3::ONE * 0.02),
				None,
				None,
			);
			self.input.spatial.set_local_transform(world_transform);
		}
		self.datamap.select = controller.trigger;
		self.datamap.grab = controller.grip;
		self.datamap.scroll = controller.stick.into();
		*self.input.datamap.lock() = Datamap::from_typed(&self.datamap).unwrap();

		let distance_calculator = |space: &Arc<Spatial>, _data: &InputDataType, field: &Field| {
			Some(field.distance(space, [0.0; 3].into()).abs())
		};

		self.capture_manager.update_capture(&self.input);
		self.capture_manager
			.set_new_capture(&self.input, distance_calculator);
		self.capture_manager.apply_capture(&self.input);

		if self.capture_manager.capture.is_some() {
			return;
		}

		let sorted_handlers = get_sorted_handlers(&self.input, distance_calculator);
		self.input.set_handler_order(sorted_handlers.iter());
	}
}

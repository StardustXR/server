use crate::{
	core::client::INTERNAL_CLIENT,
	nodes::{
		fields::FieldTrait,
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
	capture: Option<Arc<InputHandler>>,
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
			capture: None,
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
			self.material.color_tint(if self.capture.is_none() {
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

		// remove the capture when it's removed from captures list
		if let Some(capture) = &self.capture {
			if !self
				.input
				.internal_capture_requests
				.get_valid_contents()
				.contains(capture)
			{
				self.capture.take();
			}
		}
		// add the capture that's the closest if we don't have one
		if self.capture.is_none() {
			self.capture = self
				.input
				.internal_capture_requests
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
		self.input.captures.clear();
		if let Some(capture) = &self.capture {
			self.input.set_handler_order([capture].into_iter());
			self.input.captures.add_raw(capture);
			return;
		}

		// send input to all the input handlers that are the closest to the ray as possible
		self.input.set_handler_order(
			INPUT_HANDLER_REGISTRY
				.get_valid_contents()
				.into_iter()
				// filter out all the disabled handlers
				.filter(|handler| {
					let Some(node) = handler.spatial.node() else {
						return false;
					};
					node.enabled()
				})
				// filter out all the fields with disabled handlers
				.filter(|handler| {
					let Some(node) = handler.field.spatial.node() else {
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

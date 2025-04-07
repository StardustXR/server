pub mod eye_pointer;
pub mod mouse_pointer;
pub mod sk_controller;
pub mod sk_hand;

use crate::nodes::{
	fields::{Field, FieldTrait, Ray},
	input::{INPUT_HANDLER_REGISTRY, InputDataTrait, InputDataType, InputHandler, InputMethod},
	spatial::Spatial,
};
use glam::vec3;
use std::sync::Arc;

#[derive(Default)]
pub struct CaptureManager {
	pub capture: Option<Arc<InputHandler>>,
}
impl CaptureManager {
	pub fn update_capture(&mut self, method: &InputMethod) {
		if let Some(capture) = &self.capture {
			if !method
				.capture_attempts
				.get_valid_contents()
				.contains(capture)
			{
				self.capture.take();
			}
		}
	}
	pub fn set_new_capture(
		&mut self,
		method: &InputMethod,
		distance_calculator: DistanceCalculator,
	) {
		if self.capture.is_none() {
			self.capture = find_closest_capture(method, distance_calculator);
		}
	}
	pub fn apply_capture(&self, method: &InputMethod) {
		method.captures.clear();
		if let Some(capture) = &self.capture {
			method.set_handler_order([capture].into_iter());
			method.captures.add_raw(capture);
		}
	}
}

type DistanceCalculator = fn(&Arc<Spatial>, &InputDataType, &Field) -> Option<f32>;

pub fn find_closest_capture(
	method: &InputMethod,
	distance_calculator: DistanceCalculator,
) -> Option<Arc<InputHandler>> {
	method
		.capture_attempts
		.get_valid_contents()
		.into_iter()
		.filter_map(|h| {
			distance_calculator(&method.spatial, &method.data.lock(), &h.field)
				.map(|dist| (h.clone(), dist))
		})
		.min_by(|(_, dist_a), (_, dist_b)| dist_a.partial_cmp(dist_b).unwrap())
		.map(|(handler, _)| handler)
}

pub fn get_sorted_handlers(
	method: &InputMethod,
	distance_calculator: DistanceCalculator,
) -> Vec<Arc<InputHandler>> {
	INPUT_HANDLER_REGISTRY
		.get_valid_contents()
		.into_iter()
		.filter(|handler| handler.spatial.node().is_some_and(|node| node.enabled()))
		.filter(|handler| {
			handler
				.field
				.spatial
				.node()
				.is_some_and(|node| node.enabled())
		})
		.filter_map(|handler| {
			distance_calculator(&method.spatial, &method.data.lock(), &handler.field)
				.map(|distance| (vec![handler], distance))
		})
		.filter(|(_, distance)| *distance > 0.0)
		.reduce(|(mut handlers_a, distance_a), (handlers_b, distance_b)| {
			if (distance_a - distance_b).abs() < 0.001 {
				handlers_a.extend(handlers_b);
				(handlers_a, distance_a)
			} else if distance_a < distance_b {
				(handlers_a, distance_a)
			} else {
				(handlers_b, distance_b)
			}
		})
		.map(|(handlers, _)| handlers)
		.unwrap_or_default()
}

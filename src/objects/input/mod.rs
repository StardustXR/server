pub mod eye_pointer;
pub mod mouse_pointer;
pub mod oxr_controller;
pub mod oxr_hand;

use crate::nodes::{
	fields::{Field, FieldTrait, Ray},
	input::{INPUT_HANDLER_REGISTRY, InputDataTrait, InputDataType, InputHandler, InputMethod},
	spatial::Spatial,
};
use glam::vec3;
use std::{
	collections::VecDeque,
	sync::{Arc, Weak},
};

#[derive(Default)]
pub struct CaptureManager {
	pub capture: Weak<InputHandler>,
}
impl CaptureManager {
	pub fn update_capture(&mut self, method: &InputMethod) {
		if let Some(capture) = &self.capture.upgrade() {
			if !method
				.capture_attempts
				.get_valid_contents()
				.contains(capture)
			{
				self.capture = Weak::new();
			}
		}
	}
	pub fn set_new_capture(
		&mut self,
		method: &InputMethod,
		distance_calculator: DistanceCalculator,
	) {
		if self.capture.upgrade().is_none() {
			self.capture = find_closest_capture(method, distance_calculator);
		}
	}
	pub fn apply_capture(&self, method: &InputMethod) {
		method.captures.clear();
		if let Some(capture) = &self.capture.upgrade() {
			method.set_handler_order([capture].into_iter());
			method.captures.add_raw(capture);
		}
	}
}

type DistanceCalculator = fn(&Arc<Spatial>, &InputDataType, &Field) -> Option<f32>;

pub fn find_closest_capture(
	method: &InputMethod,
	distance_calculator: DistanceCalculator,
) -> Weak<InputHandler> {
	method
		.capture_attempts
		.get_valid_contents()
		.into_iter()
		.filter_map(|h| {
			distance_calculator(&method.spatial, &method.data.lock(), &h.field)
				.map(|dist| (h.clone(), dist))
		})
		.min_by(|(_, dist_a), (_, dist_b)| dist_a.partial_cmp(dist_b).unwrap())
		.map(|(handler, _)| Arc::downgrade(&handler))
		.unwrap_or_default()
}

/// sorts them greatest to least distance (so you can pop off the closest ones easily)
pub fn get_sorted_handlers(
	method: &InputMethod,
	distance_calculator: DistanceCalculator,
) -> Vec<(Arc<InputHandler>, f32)> {
	let mut handlers = INPUT_HANDLER_REGISTRY
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
				.map(|distance| (handler, distance))
		})
		.collect::<Vec<_>>();
	handlers.sort_by(|(_, dist_a), (_, dist_b)| dist_a.partial_cmp(dist_b).unwrap());
	handlers
}

use super::{DistanceLink, InputSpecialization};
use crate::nodes::fields::Field;
use crate::nodes::spatial::Spatial;
use glam::{vec3a, Mat4};
use portable_atomic::AtomicF32;
use stardust_xr::schemas::flat::{Datamap, InputDataType, Tip as FlatTip};
use std::sync::atomic::Ordering;
use std::sync::Arc;

#[derive(Default)]
pub struct Tip {
	pub radius: AtomicF32,
	pub grab: AtomicF32,
	pub select: AtomicF32,
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
			radius: self.radius.load(Ordering::Relaxed),
		})
	}
	fn serialize_datamap(&self) -> Datamap {
		let mut fbb = flexbuffers::Builder::default();
		let mut map = fbb.start_map();
		map.push("grab", self.grab.load(Ordering::Relaxed));
		map.push("select", self.select.load(Ordering::Relaxed));
		map.end_map();
		Datamap::new(fbb.view().to_vec()).unwrap()
	}
}

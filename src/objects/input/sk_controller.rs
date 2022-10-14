use crate::nodes::{
	input::{tip::Tip, InputMethod, InputType},
	spatial::Spatial,
};
use glam::Mat4;
use portable_atomic::Ordering;
use stardust_xr::values::Transform;
use std::sync::{Arc, Weak};
use stereokit::{input::Handed, StereoKit};

pub struct SkController {
	tip: Arc<InputMethod>,
	handed: Handed,
}
impl SkController {
	pub fn new(handed: Handed) -> Self {
		SkController {
			tip: InputMethod::new(
				Spatial::new(Weak::new(), None, Mat4::IDENTITY),
				InputType::Tip(Tip::default()),
			),
			handed,
		}
	}
	pub fn update(&mut self, sk: &StereoKit) {
		if let InputType::Tip(tip) = &mut *self.tip.specialization.lock() {
			let controller = sk.input_controller(self.handed);
			*self.tip.enabled.lock() = controller.tracked.is_active();
			if controller.tracked.is_active() {
				self.tip.spatial.set_local_transform_components(
					None,
					Transform {
						position: Some(controller.pose.position),
						rotation: Some(controller.pose.orientation),
						scale: None,
					},
				);
			}

			tip.select.store(controller.trigger, Ordering::Relaxed);
			tip.grab.store(controller.grip, Ordering::Relaxed);
		}
	}
}

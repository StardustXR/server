use input::{
	eye_pointer::EyePointer, mouse_pointer::MousePointer, sk_controller::SkController,
	sk_hand::SkHand,
};
use play_space::PlaySpace;
use stereokit_rust::{
	sk::{DisplayMode, MainThreadToken, Sk},
	system::{Handed, World},
	util::Device,
};

pub mod input;
pub mod play_space;

pub struct ServerObjects {
	mouse_pointer: Option<MousePointer>,
	hands: Option<(SkHand, SkHand)>,
	controllers: Option<(SkController, SkController)>,
	eye_pointer: Option<EyePointer>,
	play_space: Option<PlaySpace>,
}
impl ServerObjects {
	pub fn new(intentional_flatscreen: bool, sk: &Sk) -> ServerObjects {
		ServerObjects {
			mouse_pointer: intentional_flatscreen
				.then(MousePointer::new)
				.transpose()
				.unwrap(),
			hands: (!intentional_flatscreen)
				.then(|| {
					let left = SkHand::new(Handed::Left).ok();
					let right = SkHand::new(Handed::Right).ok();
					left.zip(right)
				})
				.flatten(),
			controllers: (!intentional_flatscreen)
				.then(|| {
					let left = SkController::new(Handed::Left).ok();
					let right = SkController::new(Handed::Right).ok();
					left.zip(right)
				})
				.flatten(),
			eye_pointer: (sk.get_active_display_mode() == DisplayMode::MixedReality
				&& Device::has_eye_gaze())
			.then(EyePointer::new)
			.transpose()
			.unwrap(),
			play_space: World::has_bounds().then(|| PlaySpace::new().ok()).flatten(),
		}
	}

	pub fn update(&mut self, sk: &Sk, token: &MainThreadToken) {
		if let Some(mouse_pointer) = self.mouse_pointer.as_mut() {
			mouse_pointer.update();
		}
		if let Some((left_hand, right_hand)) = self.hands.as_mut() {
			left_hand.update(&sk, token);
			right_hand.update(&sk, token);
		}
		if let Some((left_controller, right_controller)) = self.controllers.as_mut() {
			left_controller.update(token);
			right_controller.update(token);
		}
		if let Some(eye_pointer) = self.eye_pointer.as_ref() {
			eye_pointer.update();
		}
		if let Some(play_space) = self.play_space.as_ref() {
			play_space.update();
		}
	}
}

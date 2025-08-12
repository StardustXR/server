use super::toplevel::Toplevel;
use crate::{
        core::error::Result,
        nodes::{
                drawable::model::ModelPart,
                items::panel::{Backend, Geometry, PanelItemInitData, SurfaceId, ToplevelInfo},
        },
        wayland::{Message, core::surface::Surface},
};
use mint::Vector2;
use parking_lot::Mutex;
use std::{collections::HashMap, sync::{Arc, Weak}};
use tracing;

#[derive(Debug)]
pub struct XdgBackend {
        toplevel: Weak<Toplevel>,
        children: Mutex<HashMap<u64, Weak<Surface>>>,
}

impl XdgBackend {
        pub fn new(toplevel: Arc<Toplevel>) -> Self {
                Self {
                        toplevel: Arc::downgrade(&toplevel),
                        children: Mutex::new(HashMap::new()),
                }
        }

	// Since XdgBackend is created and owned by Mapped which is owned by Toplevel,
	// we can safely assume the Toplevel reference will always be valid
	fn toplevel(&self) -> Arc<Toplevel> {
		self.toplevel
			.upgrade()
			.expect("Toplevel should always be valid while XdgBackend exists")
	}

        fn surface_from_id(&self, id: SurfaceId) -> Option<Arc<Surface>> {
                match id {
                        SurfaceId::Toplevel(_) => Some(self.toplevel().surface()),
                        SurfaceId::Child(uid) => self.children.lock().get(&uid).and_then(Weak::upgrade),
                }
        }

        pub fn register_child(&self, id: u64, surface: &Arc<Surface>) {
                self.children.lock().insert(id, Arc::downgrade(surface));
        }

        pub fn unregister_child(&self, id: u64) {
                self.children.lock().remove(&id);
        }
}

impl Backend for XdgBackend {
	fn start_data(&self) -> Result<PanelItemInitData> {
		let surface_state = self.toplevel().surface().current_state();

		let size = surface_state
			.buffer
			.map(|b| [b.buffer.size().x as u32, b.buffer.size().y as u32].into())
			.unwrap_or([0; 2].into());
		let toplevel = ToplevelInfo {
			parent: self.toplevel().parent(),
			title: self.toplevel().title(),
			app_id: self.toplevel().app_id(),
			size,
			min_size: surface_state
				.min_size
				.map(|v| [v.x as f32, v.y as f32].into()),
			max_size: surface_state
				.max_size
				.map(|v| [v.x as f32, v.y as f32].into()),
			logical_rectangle: surface_state.geometry.unwrap_or(Geometry {
				origin: [0; 2].into(),
				size,
			}),
		};

		Ok(PanelItemInitData {
			cursor: None,
			toplevel,
			children: vec![],
			pointer_grab: None,
			keyboard_grab: None,
		})
	}

	fn apply_cursor_material(&self, _model_part: &Arc<ModelPart>) {}
	fn apply_surface_material(&self, surface: SurfaceId, model_part: &Arc<ModelPart>) {
		if let Some(surface) = self.surface_from_id(surface) {
			surface.apply_material(model_part);
		}
	}

	fn close_toplevel(&self) {
		let _ = self
			.toplevel()
			.surface()
			.message_sink
			.send(Message::CloseToplevel(self.toplevel().clone()));
	}

	fn auto_size_toplevel(&self) {
		let _ = self
			.toplevel()
			.surface()
			.message_sink
			.send(Message::ResizeToplevel {
				toplevel: self.toplevel().clone(),
				size: None,
			});
	}

	fn set_toplevel_size(&self, size: Vector2<u32>) {
		let _ = self
			.toplevel()
			.surface()
			.message_sink
			.send(Message::ResizeToplevel {
				toplevel: self.toplevel().clone(),
				size: Some(size),
			});
	}

	fn set_toplevel_focused_visuals(&self, focused: bool) {
		let _ = self
			.toplevel()
			.surface()
			.message_sink
			.send(Message::SetToplevelVisualActive {
				toplevel: self.toplevel().clone(),
				active: focused,
			});
	}

	fn pointer_motion(&self, surface: &SurfaceId, position: Vector2<f32>) {
		if let Some(surface) = self.surface_from_id(surface.clone()) {
			let _ = self.toplevel().surface().message_sink.send(Message::Seat(
				crate::wayland::core::seat::SeatMessage::PointerMotion { surface, position },
			));
		}
	}

	fn pointer_button(&self, surface: &SurfaceId, button: u32, pressed: bool) {
		if let Some(surface) = self.surface_from_id(surface.clone()) {
			let _ = self.toplevel().surface().message_sink.send(Message::Seat(
				crate::wayland::core::seat::SeatMessage::PointerButton {
					surface,
					button,
					pressed,
				},
			));
		}
	}

	fn pointer_scroll(
		&self,
		surface: &SurfaceId,
		scroll_distance: Option<Vector2<f32>>,
		scroll_steps: Option<Vector2<f32>>,
	) {
		if let Some(surface) = self.surface_from_id(surface.clone()) {
			let _ = self.toplevel().surface().message_sink.send(Message::Seat(
				crate::wayland::core::seat::SeatMessage::PointerScroll {
					surface,
					scroll_distance,
					scroll_steps,
				},
			));
		}
	}

	fn keyboard_key(&self, surface: &SurfaceId, keymap_id: u64, key: u32, pressed: bool) {
		tracing::debug!(
			"Backend: Keyboard key {} {}",
			key,
			if pressed { "pressed" } else { "released" }
		);
		if let Some(surface) = self.surface_from_id(surface.clone()) {
			let _ = self.toplevel().surface().message_sink.send(Message::Seat(
				crate::wayland::core::seat::SeatMessage::KeyboardKey {
					surface,
					keymap_id,
					key,
					pressed,
				},
			));
		}
	}

	fn touch_down(&self, surface: &SurfaceId, id: u32, position: Vector2<f32>) {
		tracing::debug!(
			"Backend: Touch down {} at ({}, {})",
			id,
			position.x,
			position.y
		);
		if let Some(surface) = self.surface_from_id(surface.clone()) {
			let _ = self.toplevel().surface().message_sink.send(Message::Seat(
				crate::wayland::core::seat::SeatMessage::TouchDown {
					surface,
					id,
					position,
				},
			));
		}
	}

	fn touch_move(&self, id: u32, position: Vector2<f32>) {
		tracing::debug!(
			"Backend: Touch move {} to ({}, {})",
			id,
			position.x,
			position.y
		);
		let surface = self.toplevel().surface();
		let _ = surface.message_sink.send(Message::Seat(
			crate::wayland::core::seat::SeatMessage::TouchMove { id, position },
		));
	}

	fn touch_up(&self, id: u32) {
		tracing::debug!("Backend: Touch up {}", id);
		let surface = self.toplevel().surface();
		let _ = surface.message_sink.send(Message::Seat(
			crate::wayland::core::seat::SeatMessage::TouchUp { id },
		));
	}

	fn reset_input(&self) {
		tracing::debug!("Backend: Reset input");
		let surface = self.toplevel().surface();
		let _ = surface.message_sink.send(Message::Seat(
			crate::wayland::core::seat::SeatMessage::Reset,
		));
	}
}

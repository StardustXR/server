use super::toplevel::Toplevel;
use crate::{
	core::{error::Result, task},
	nodes::{
		drawable::model::ModelPart,
		items::panel::{
			Backend, ChildInfo, Geometry, PanelItem, PanelItemInitData, SurfaceId, ToplevelInfo,
		},
	},
	wayland::{
		Message,
		core::{
			seat::{Seat, SeatMessage},
			surface::Surface,
		},
	},
};
use dashmap::DashMap;
use mint::Vector2;
use std::sync::Arc;
use std::sync::Weak;
use tracing;

#[derive(Debug)]
pub struct XdgBackend {
	seat: Weak<Seat>,
	toplevel: Weak<Toplevel>,
	pub children: DashMap<u64, (Weak<Surface>, ChildInfo)>,
}

impl XdgBackend {
	pub fn new(seat: &Arc<Seat>, toplevel: &Arc<Toplevel>) -> Self {
		Self {
			seat: Arc::downgrade(seat),
			toplevel: Arc::downgrade(toplevel),
			children: DashMap::new(),
		}
	}

	// Since XdgBackend is created and owned by Mapped which is owned by Toplevel,
	// we can safely assume the Toplevel reference will always be valid
	fn toplevel(&self) -> Arc<Toplevel> {
		self.toplevel
			.upgrade()
			.expect("Toplevel should always be valid while XdgBackend exists")
	}

	pub fn panel_item(&self) -> Option<Arc<PanelItem<XdgBackend>>> {
		self.toplevel().wl_surface().panel_item.lock().upgrade()
	}

	fn surface_from_id(&self, id: &SurfaceId) -> Option<Arc<Surface>> {
		match id {
			SurfaceId::Toplevel(_) => Some(self.toplevel().wl_surface().clone()),
			SurfaceId::Child(id) => self.children.get(id).as_deref().and_then(|c| c.0.upgrade()),
		}
	}

	pub fn add_child(&self, surface: &Arc<Surface>, info: ChildInfo) {
		let Some(SurfaceId::Child(id)) = surface.surface_id.get().cloned() else {
			return;
		};
		self.children
			.insert(id, (Arc::downgrade(surface), info.clone()));

		let Some(panel_item) = self.panel_item() else {
			tracing::error!("Couldn't find panel item in add_child");
			return;
		};
		panel_item.create_child(id, &info);
	}

	pub fn reposition_child(&self, surface: &Arc<Surface>, geometry: Geometry) {
		let Some(SurfaceId::Child(id)) = surface.surface_id.get() else {
			return;
		};

		if let Some(mut child) = self.children.get_mut(id) {
			child.1.geometry = geometry;
		}
		let Some(panel_item) = self.panel_item() else {
			tracing::error!("Couldn't find panel item in reposition_child");
			return;
		};
		panel_item.reposition_child(*id, &geometry);
	}

	pub fn update_child_z_order(&self, surface: &Arc<Surface>, z_order: i32) {
		let Some(SurfaceId::Child(id)) = surface.surface_id.get() else {
			return;
		};

		if let Some(mut child) = self.children.get_mut(id) {
			child.1.z_order = z_order;
			let info = child.1.clone();
			drop(child);

			let Some(panel_item) = self.panel_item() else {
				tracing::error!("Couldn't find panel item in update_child_z_order");
				return;
			};
			panel_item.reposition_child(*id, &info.geometry);
		}
	}

	pub fn remove_child(&self, surface: &Surface) {
		let Some(SurfaceId::Child(id)) = surface.surface_id.get() else {
			return;
		};
		self.children.remove(id);

		let Some(panel_item) = self.panel_item() else {
			tracing::error!("Couldn't find panel item in remove_child");
			return;
		};
		panel_item.destroy_child(*id);
	}
}
impl Backend for XdgBackend {
	fn start_data(&self) -> Result<PanelItemInitData> {
		let top_level = self.toplevel();
		let surface = top_level.wl_surface();
		let state_lock = surface.state_lock();
		let surface_state = state_lock.current();

		let size = surface_state
			.buffer
			.as_ref()
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

	fn apply_cursor_material(&self, model_part: &Arc<ModelPart>) {
		let model_part = model_part.clone();
		let Some(seat) = self.seat.upgrade() else {
			return;
		};
		let _ = task::new(|| "Apply cursor material", async move {
			let Some(cursor) = seat.cursor_surface().await else {
				return;
			};
			cursor.apply_material(&model_part);
		});
	}
	fn apply_surface_material(&self, surface: SurfaceId, model_part: &Arc<ModelPart>) {
		if let Some(surface) = self.surface_from_id(&surface) {
			surface.apply_material(model_part);
		}
	}

	fn close_toplevel(&self) {
		let _ = self
			.toplevel()
			.wl_surface()
			.message_sink
			.send(Message::CloseToplevel(self.toplevel().clone()));
	}

	fn auto_size_toplevel(&self) {
		let _ = self
			.toplevel()
			.wl_surface()
			.message_sink
			.send(Message::ResizeToplevel {
				toplevel: self.toplevel().clone(),
				size: None,
			});
	}

	fn set_toplevel_size(&self, size: Vector2<u32>) {
		let _ = self
			.toplevel()
			.wl_surface()
			.message_sink
			.send(Message::ResizeToplevel {
				toplevel: self.toplevel().clone(),
				size: Some(size),
			});
	}

	fn set_toplevel_focused_visuals(&self, focused: bool) {
		let _ = self
			.toplevel()
			.wl_surface()
			.message_sink
			.send(Message::SetToplevelVisualActive {
				toplevel: self.toplevel().clone(),
				active: focused,
			});
	}

	fn pointer_motion(&self, surface: &SurfaceId, position: Vector2<f32>) {
		if let Some(surface) = self.surface_from_id(surface) {
			let _ = self
				.toplevel()
				.wl_surface()
				.message_sink
				.send(Message::Seat(SeatMessage::PointerMotion {
					surface,
					position,
				}));
		}
	}

	fn pointer_button(&self, surface: &SurfaceId, button: u32, pressed: bool) {
		if let Some(surface) = self.surface_from_id(surface) {
			let _ = self
				.toplevel()
				.wl_surface()
				.message_sink
				.send(Message::Seat(SeatMessage::PointerButton {
					surface,
					button,
					pressed,
				}));
		}
	}

	fn pointer_scroll(
		&self,
		surface: &SurfaceId,
		scroll_distance: Option<Vector2<f32>>,
		scroll_steps: Option<Vector2<f32>>,
	) {
		if let Some(surface) = self.surface_from_id(surface) {
			let _ = self
				.toplevel()
				.wl_surface()
				.message_sink
				.send(Message::Seat(SeatMessage::PointerScroll {
					surface,
					scroll_distance,
					scroll_steps,
				}));
		}
	}

	fn keyboard_key(&self, surface: &SurfaceId, keymap_id: u64, key: u32, pressed: bool) {
		tracing::debug!(
			"Backend: Keyboard key {} {}",
			key,
			if pressed { "pressed" } else { "released" }
		);
		if let Some(surface) = self.surface_from_id(surface) {
			let _ = self
				.toplevel()
				.wl_surface()
				.message_sink
				.send(Message::Seat(SeatMessage::KeyboardKey {
					surface,
					keymap_id,
					key,
					pressed,
				}));
		}
	}

	fn touch_down(&self, surface: &SurfaceId, id: u32, position: Vector2<f32>) {
		tracing::debug!(
			"Backend: Touch down {} at ({}, {})",
			id,
			position.x,
			position.y
		);
		if let Some(surface) = self.surface_from_id(surface) {
			let _ = self
				.toplevel()
				.wl_surface()
				.message_sink
				.send(Message::Seat(SeatMessage::TouchDown {
					surface,
					id,
					position,
				}));
		}
	}

	fn touch_move(&self, id: u32, position: Vector2<f32>) {
		tracing::debug!(
			"Backend: Touch move {} to ({}, {})",
			id,
			position.x,
			position.y
		);
		let toplevel = self.toplevel();
		let _ = toplevel
			.wl_surface()
			.message_sink
			.send(Message::Seat(SeatMessage::TouchMove { id, position }));
	}

	fn touch_up(&self, id: u32) {
		tracing::debug!("Backend: Touch up {}", id);
		let toplevel = self.toplevel();
		let _ = toplevel
			.wl_surface()
			.message_sink
			.send(Message::Seat(SeatMessage::TouchUp { id }));
	}

	fn reset_input(&self) {
		tracing::debug!("Backend: Reset input");
		let toplevel = self.toplevel();
		let _ = toplevel
			.wl_surface()
			.message_sink
			.send(Message::Seat(SeatMessage::Reset));
	}
}

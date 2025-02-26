use super::toplevel::Toplevel;
use crate::{
	core::error::Result,
	nodes::{
		drawable::model::ModelPart,
		items::panel::{Backend, Geometry, PanelItemInitData, SurfaceId, ToplevelInfo},
	},
	wayland::core::surface::Surface,
};
use mint::Vector2;
use std::sync::Arc;

#[derive(Debug)]
pub struct XdgBackend {
	pub toplevel: Arc<Toplevel>,
}
impl XdgBackend {
	fn surface_from_id(&self, id: SurfaceId) -> Option<Arc<Surface>> {
		match id {
			SurfaceId::Toplevel(_) => Some(self.toplevel.wl_surface.clone()),
			SurfaceId::Child(_) => None,
		}
	}
}
impl Backend for XdgBackend {
	fn start_data(&self) -> Result<PanelItemInitData> {
		let surface_state = self.toplevel.wl_surface.current_state();

		let size = surface_state
			.buffer
			.map(|b| [b.size.x as u32, b.size.y as u32].into())
			.unwrap_or([0; 2].into());
		let toplevel = ToplevelInfo {
			parent: self.toplevel.parent(),
			title: self.toplevel.title(),
			app_id: self.toplevel.app_id(),
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
		let Some(surface) = self.surface_from_id(surface) else {
			return;
		};
		surface.apply_material(model_part);
	}
	fn close_toplevel(&self) {
		tracing::info!("closing toplevel");
		let _ = self
			.toplevel
			.wl_surface
			.message_sink
			.send(crate::wayland::Message::CloseToplevel(
				self.toplevel.clone(),
			));
	}

	fn auto_size_toplevel(&self) {}
	fn set_toplevel_size(&self, _size: Vector2<u32>) {}
	fn set_toplevel_focused_visuals(&self, _focused: bool) {}

	fn pointer_motion(&self, _surface: &SurfaceId, _position: Vector2<f32>) {}
	fn pointer_button(&self, _surface: &SurfaceId, _button: u32, _pressed: bool) {}
	fn pointer_scroll(
		&self,
		_surface: &SurfaceId,
		_scroll_distance: Option<Vector2<f32>>,
		_scroll_steps: Option<Vector2<f32>>,
	) {
	}

	fn keyboard_key(&self, _surface: &SurfaceId, _keymap_id: u64, _key: u32, _pressed: bool) {}

	fn touch_down(&self, _surface: &SurfaceId, _id: u32, _position: Vector2<f32>) {}
	fn touch_move(&self, _id: u32, _position: Vector2<f32>) {}
	fn touch_up(&self, _id: u32) {}

	fn reset_input(&self) {}
}

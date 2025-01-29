use super::toplevel::Toplevel;
use crate::{
	core::error::Result,
	nodes::{
		drawable::model::ModelPart,
		items::panel::{Backend, PanelItemInitData, SurfaceId},
	},
};
use mint::Vector2;
use std::sync::{Arc, OnceLock};

#[derive(Default)]
pub struct XdgBackend {
	toplevel: OnceLock<Arc<Toplevel>>,
}
impl Backend for XdgBackend {
	fn start_data(&self) -> Result<PanelItemInitData> {
		Ok(PanelItemInitData {
			cursor: None,
			toplevel: self.toplevel.get().unwrap().info.lock().clone(),
			children: vec![],
			pointer_grab: None,
			keyboard_grab: None,
		})
	}

	fn apply_cursor_material(&self, model_part: &Arc<ModelPart>) {
		todo!()
	}
	fn apply_surface_material(&self, surface: SurfaceId, model_part: &Arc<ModelPart>) {
		todo!()
	}
	fn close_toplevel(&self) {
		todo!()
	}

	fn auto_size_toplevel(&self) {
		todo!()
	}
	fn set_toplevel_size(&self, size: Vector2<u32>) {
		todo!()
	}
	fn set_toplevel_focused_visuals(&self, focused: bool) {
		todo!()
	}

	fn pointer_motion(&self, surface: &SurfaceId, position: Vector2<f32>) {
		todo!()
	}
	fn pointer_button(&self, surface: &SurfaceId, button: u32, pressed: bool) {
		todo!()
	}
	fn pointer_scroll(
		&self,
		surface: &SurfaceId,
		scroll_distance: Option<Vector2<f32>>,
		scroll_steps: Option<Vector2<f32>>,
	) {
		todo!()
	}

	fn keyboard_key(&self, surface: &SurfaceId, keymap_id: u64, key: u32, pressed: bool) {
		todo!()
	}

	fn touch_down(&self, surface: &SurfaceId, id: u32, position: Vector2<f32>) {
		todo!()
	}
	fn touch_move(&self, id: u32, position: Vector2<f32>) {
		todo!()
	}
	fn touch_up(&self, id: u32) {
		todo!()
	}

	fn reset_input(&self) {
		todo!()
	}
}

use super::{popup::PopupData, surface::XdgSurfaceData, ToplevelData};
use crate::{
	nodes::{
		drawable::model::ModelPart,
		items::panel::{Backend, ChildInfo, PanelItem, PanelItemInitData, SurfaceID},
	},
	wayland::{seat::SeatWrapper, state::ClientState, surface::CoreSurface, utils, SERIAL_COUNTER},
};
use color_eyre::eyre::{eyre, Result};
use mint::Vector2;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use smithay::reexports::{
	wayland_protocols::xdg::shell::server::xdg_toplevel::XdgToplevel,
	wayland_server::{protocol::wl_surface::WlSurface, Resource, Weak as WlWeak},
};
use std::sync::Arc;
use tracing::debug;

pub struct XdgToplevelState {
	pub fullscreen: bool,
	pub activated: bool,
}

pub struct XdgBackend {
	toplevel: WlWeak<XdgToplevel>,
	toplevel_wl_surface: WlWeak<WlSurface>,
	pub toplevel_state: Mutex<XdgToplevelState>,
	popups: Mutex<FxHashMap<String, WlWeak<WlSurface>>>,
	pub seat: Arc<SeatWrapper>,
}
impl XdgBackend {
	pub fn create(
		toplevel_wl_surface: WlSurface,
		toplevel: XdgToplevel,
		seat: Arc<SeatWrapper>,
	) -> Self {
		XdgBackend {
			toplevel: toplevel.downgrade(),
			toplevel_wl_surface: toplevel_wl_surface.downgrade(),
			toplevel_state: Mutex::new(XdgToplevelState {
				fullscreen: false,
				activated: false,
			}),
			popups: Mutex::new(FxHashMap::default()),
			seat,
		}
	}
	fn wl_surface_from_id(&self, id: &SurfaceID) -> Option<WlSurface> {
		match id {
			SurfaceID::Cursor => self
				.seat
				.cursor_info_rx
				.borrow()
				.surface
				.clone()?
				.upgrade()
				.ok(),
			SurfaceID::Toplevel => self.toplevel_wl_surface(),
			SurfaceID::Child(popup) => {
				let popups = self.popups.lock();
				popups.get(popup)?.upgrade().ok()
			}
		}
	}
	fn toplevel_wl_surface(&self) -> Option<WlSurface> {
		self.toplevel_wl_surface.upgrade().ok()
	}

	pub fn configure(&self, size: Option<Vector2<u32>>) {
		let Ok(xdg_toplevel) = self.toplevel.upgrade() else {
			return;
		};
		let Some(wl_surface) = self.toplevel_wl_surface() else {
			return;
		};
		let Some(xdg_surface_data) = wl_surface.data::<XdgSurfaceData>() else {
			return;
		};
		let Some(core_surface) = CoreSurface::from_wl_surface(&wl_surface) else {
			return;
		};
		let Some(surface_size) = core_surface.size() else {
			return;
		};

		xdg_toplevel.configure(
			size.unwrap_or(surface_size).x as i32,
			size.unwrap_or(surface_size).y as i32,
			self.states()
				.into_iter()
				.flat_map(|state| state.to_ne_bytes())
				.collect(),
		);
		xdg_surface_data.xdg_surface.configure(SERIAL_COUNTER.inc());
		self.flush_client();
	}
	fn states(&self) -> Vec<u32> {
		let mut states = vec![1, 5, 6, 7, 8]; // maximized always and tiled
		let toplevel_state = self.toplevel_state.lock();
		if toplevel_state.fullscreen {
			states.push(2);
		}
		if toplevel_state.activated {
			states.push(4);
		}
		states
	}

	pub fn new_popup(
		&self,
		panel_item: &PanelItem<XdgBackend>,
		popup_wl_surface: &WlSurface,
		data: &PopupData,
	) {
		self.popups
			.lock()
			.insert(data.uid.clone(), popup_wl_surface.downgrade());

		let Some(geometry) = data.geometry() else {
			return;
		};
		panel_item.new_child(
			&data.uid,
			ChildInfo {
				parent: utils::get_data::<SurfaceID>(&data.parent())
					.unwrap()
					.as_ref()
					.clone(),
				geometry,
			},
		)
	}
	pub fn reposition_popup(&self, panel_item: &PanelItem<XdgBackend>, popup_state: &PopupData) {
		let Some(geometry) = popup_state.geometry() else {
			return;
		};
		panel_item.reposition_child(&popup_state.uid, geometry)
	}
	pub fn drop_popup(&self, panel_item: &PanelItem<XdgBackend>, uid: &str) {
		panel_item.drop_child(uid);
	}

	fn child_data(&self) -> FxHashMap<String, ChildInfo> {
		FxHashMap::from_iter(self.popups.lock().iter().filter_map(|(uid, v)| {
			let wl_surface = v.upgrade().ok()?;
			let popup_data = utils::get_data::<PopupData>(&wl_surface)?;
			let parent = utils::get_data::<SurfaceID>(&popup_data.parent())?
				.as_ref()
				.clone();
			let geometry = utils::get_data::<XdgSurfaceData>(&wl_surface)?
				.geometry
				.lock()
				.clone()?;
			Some((uid.clone(), ChildInfo { parent, geometry }))
		}))
	}

	fn flush_client(&self) {
		let Some(client) = self.toplevel_wl_surface().and_then(|s| s.client()) else {
			return;
		};
		if let Some(client_state) = client.get_data::<ClientState>() {
			client_state.flush();
		}
	}
}
impl Drop for XdgBackend {
	fn drop(&mut self) {
		debug!("Dropped panel item gracefully");
	}
}
impl Backend for XdgBackend {
	fn start_data(&self) -> Result<PanelItemInitData> {
		let toplevel = self.toplevel_wl_surface();
		let toplevel_data = toplevel.as_ref().and_then(utils::get_data::<ToplevelData>);
		let toplevel_data = toplevel_data
			.as_deref()
			.clone()
			.ok_or_else(|| eyre!("Could not get toplevel"))?;

		Ok(PanelItemInitData {
			cursor: (*self.seat.cursor_info_rx.borrow()).cursor_data(),
			toplevel: toplevel_data.into(),
			children: self.child_data(),
			pointer_grab: None,
			keyboard_grab: None,
		})
	}
	fn surface_alive(&self, surface: &SurfaceID) -> bool {
		match surface {
			SurfaceID::Cursor => true,
			SurfaceID::Toplevel => true,
			SurfaceID::Child(c) => self.popups.lock().contains_key(c),
		}
	}

	fn apply_surface_material(&self, surface: SurfaceID, model_part: &Arc<ModelPart>) {
		let Some(wl_surface) = self.wl_surface_from_id(&surface) else {
			return;
		};
		let Some(core_surface) = CoreSurface::from_wl_surface(&wl_surface) else {
			return;
		};

		core_surface.apply_material(model_part);
	}

	fn close_toplevel(&self) {
		let Ok(xdg_toplevel) = self.toplevel.upgrade() else {
			return;
		};
		xdg_toplevel.close();
	}
	fn auto_size_toplevel(&self) {
		self.configure(Some([0, 0].into()));
	}
	fn set_toplevel_size(&self, size: Vector2<u32>) {
		self.configure(Some(size));
	}
	fn set_toplevel_focused_visuals(&self, focused: bool) {
		self.toplevel_state.lock().activated = focused;
		self.configure(None);
	}

	fn pointer_motion(&self, surface: &SurfaceID, position: Vector2<f32>) {
		let Some(surface) = self.wl_surface_from_id(surface) else {
			return;
		};
		self.seat.pointer_motion(surface, position)
	}
	fn pointer_button(&self, _surface: &SurfaceID, button: u32, pressed: bool) {
		self.seat.pointer_button(button, pressed)
	}
	fn pointer_scroll(
		&self,
		_surface: &SurfaceID,
		scroll_distance: Option<Vector2<f32>>,
		scroll_steps: Option<Vector2<f32>>,
	) {
		self.seat.pointer_scroll(scroll_distance, scroll_steps)
	}

	fn keyboard_keys(&self, surface: &SurfaceID, keymap_id: &str, keys: Vec<i32>) {
		let Some(surface) = self.wl_surface_from_id(surface) else {
			return;
		};
		self.seat.keyboard_keys(surface, keymap_id, keys)
	}

	fn touch_down(&self, surface: &SurfaceID, id: u32, position: Vector2<f32>) {
		let Some(surface) = self.wl_surface_from_id(surface) else {
			return;
		};
		self.seat.touch_down(surface, id, position)
	}
	fn touch_move(&self, id: u32, position: Vector2<f32>) {
		self.seat.touch_move(id, position)
	}
	fn touch_up(&self, id: u32) {
		self.seat.touch_up(id)
	}
	fn reset_touches(&self) {
		self.seat.reset_touches()
	}
}

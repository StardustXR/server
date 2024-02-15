use super::{popup::PopupData, surface::XdgSurfaceData, ToplevelData};
use crate::{
	nodes::{
		data::KEYMAPS,
		drawable::model::ModelPart,
		items::panel::{Backend, ChildInfo, PanelItem, PanelItemInitData, SurfaceID},
	},
	wayland::{
		seat::{CursorInfo, KeyboardEvent, PointerEvent, SeatData},
		state::ClientState,
		surface::CoreSurface,
		utils, SERIAL_COUNTER,
	},
};
use color_eyre::eyre::{eyre, Result};
use mint::Vector2;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use smithay::reexports::{
	wayland_protocols::xdg::shell::server::xdg_toplevel::XdgToplevel,
	wayland_server::{protocol::wl_surface::WlSurface, Resource, Weak},
};
use std::sync::Arc;
use tokio::sync::watch;
use tracing::debug;

pub struct XdgToplevelState {
	pub fullscreen: bool,
	pub activated: bool,
}

pub struct XdgBackend {
	toplevel: Weak<XdgToplevel>,
	toplevel_wl_surface: Weak<WlSurface>,
	pub toplevel_state: Mutex<XdgToplevelState>,
	popups: Mutex<FxHashMap<String, Weak<WlSurface>>>,
	pub cursor: watch::Receiver<Option<CursorInfo>>,
	pub seat: Arc<SeatData>,
	pointer_grab: Mutex<Option<SurfaceID>>,
	keyboard_grab: Mutex<Option<SurfaceID>>,
}
impl XdgBackend {
	pub fn create(
		toplevel_wl_surface: WlSurface,
		toplevel: XdgToplevel,
		seat: Arc<SeatData>,
	) -> Self {
		let cursor = seat.new_surface(&toplevel_wl_surface);
		XdgBackend {
			toplevel: toplevel.downgrade(),
			toplevel_wl_surface: toplevel_wl_surface.downgrade(),
			toplevel_state: Mutex::new(XdgToplevelState {
				fullscreen: false,
				activated: false,
			}),
			popups: Mutex::new(FxHashMap::default()),
			cursor,
			seat,
			pointer_grab: Mutex::new(None),
			keyboard_grab: Mutex::new(None),
		}
	}
	fn wl_surface_from_id(&self, id: &SurfaceID) -> Option<WlSurface> {
		match id {
			SurfaceID::Cursor => self.cursor.borrow().as_ref()?.surface.upgrade().ok(),
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
		let Some(popup) = self.popups.lock().remove(uid) else {
			return;
		};
		let Some(wl_surface) = popup.upgrade().ok() else {
			return;
		};
		self.seat.drop_surface(&wl_surface);
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

		let pointer_grab = self.pointer_grab.lock().clone();
		let keyboard_grab = self.keyboard_grab.lock().clone();

		Ok(PanelItemInitData {
			cursor: self.cursor.borrow().as_ref().and_then(|c| c.cursor_data()),
			toplevel: toplevel_data.into(),
			children: self.child_data(),
			pointer_grab,
			keyboard_grab,
		})
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
		self.seat
			.pointer_event(&surface, PointerEvent::Motion(position));
	}
	fn pointer_button(&self, surface: &SurfaceID, button: u32, pressed: bool) {
		let Some(surface) = self.wl_surface_from_id(surface) else {
			return;
		};
		self.seat.pointer_event(
			&surface,
			PointerEvent::Button {
				button,
				state: if pressed { 1 } else { 0 },
			},
		)
	}
	fn pointer_scroll(
		&self,
		surface: &SurfaceID,
		scroll_distance: Option<Vector2<f32>>,
		scroll_steps: Option<Vector2<f32>>,
	) {
		let Some(surface) = self.wl_surface_from_id(surface) else {
			return;
		};
		self.seat.pointer_event(
			&surface,
			PointerEvent::Scroll {
				axis_continuous: scroll_distance,
				axis_discrete: scroll_steps,
			},
		)
	}

	fn keyboard_keys(&self, surface: &SurfaceID, keymap_id: &str, keys: Vec<i32>) {
		let Some(surface) = self.wl_surface_from_id(surface) else {
			return;
		};
		let keymaps = KEYMAPS.lock();
		let Some(keymap) = keymaps.get(keymap_id).cloned() else {
			return;
		};
		if self.seat.set_keymap(keymap, vec![surface.clone()]).is_err() {
			return;
		}
		for key in keys {
			self.seat.keyboard_event(
				&surface,
				KeyboardEvent::Key {
					key: key.abs() as u32,
					state: key > 0,
				},
			);
		}
	}

	fn touch_down(&self, surface: &SurfaceID, id: u32, position: Vector2<f32>) {
		let Some(surface) = self.wl_surface_from_id(surface) else {
			return;
		};
		self.seat.touch_down(&surface, id, position)
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

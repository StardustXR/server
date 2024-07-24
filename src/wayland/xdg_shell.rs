use super::{
	seat::{handle_cursor, SeatWrapper},
	state::{ClientState, WaylandState},
	surface::CoreSurface,
	utils::WlSurfaceExt,
};
use crate::nodes::{
	drawable::model::ModelPart,
	items::panel::{
		Backend, ChildInfo, Geometry, PanelItem, PanelItemInitData, SurfaceId, ToplevelInfo,
	},
};
use color_eyre::eyre::Result;
use mint::Vector2;
use parking_lot::Mutex;
use rand::Rng;
use rustc_hash::FxHashMap;
use smithay::{
	delegate_xdg_shell,
	reexports::{
		wayland_protocols::xdg::{
			decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode,
			shell::server::xdg_toplevel::{ResizeEdge, State},
		},
		wayland_server::{
			protocol::{wl_output::WlOutput, wl_seat::WlSeat, wl_surface::WlSurface},
			Resource,
		},
	},
	utils::{Logical, Rectangle, Serial},
	wayland::{
		compositor::{self, add_post_commit_hook},
		shell::xdg::{
			PopupSurface, PositionerState, ShellClient, SurfaceCachedState, ToplevelSurface,
			XdgShellHandler, XdgShellState, XdgToplevelSurfaceData,
		},
	},
};
use std::sync::{Arc, Weak};
use tracing::warn;

impl From<Rectangle<i32, Logical>> for Geometry {
	fn from(value: Rectangle<i32, Logical>) -> Self {
		Geometry {
			origin: [value.loc.x, value.loc.y].into(),
			size: [value.size.w as u32, value.size.h as u32].into(),
		}
	}
}

fn surface_panel_item(wl_surface: &WlSurface) -> Option<Arc<PanelItem<XdgBackend>>> {
	let panel_item = wl_surface
		.get_data::<Weak<PanelItem<XdgBackend>>>()
		.as_ref()
		.and_then(Weak::upgrade);
	if panel_item.is_none() {
		warn!("Couldn't get panel item");
	}
	panel_item
}

impl XdgShellHandler for WaylandState {
	fn xdg_shell_state(&mut self) -> &mut XdgShellState {
		&mut self.xdg_shell
	}

	fn new_client(&mut self, _client: ShellClient) {}
	fn client_destroyed(&mut self, _client: ShellClient) {}

	fn new_toplevel(&mut self, toplevel: ToplevelSurface) {
		toplevel.wl_surface().insert_data(SurfaceId::Toplevel(()));
		toplevel.with_pending_state(|s| {
			s.decoration_mode = Some(Mode::ServerSide);
			s.states.set(State::TiledTop);
			s.states.set(State::TiledBottom);
			s.states.set(State::TiledLeft);
			s.states.set(State::TiledRight);
			s.states.set(State::Maximized);
			s.states.unset(State::Fullscreen);
		});
		toplevel.send_configure();
		toplevel
			.wl_surface()
			.insert_data(Mutex::new(Vector2::from([0_u32; 2])));

		CoreSurface::add_to(toplevel.wl_surface());
		add_post_commit_hook(
			toplevel.wl_surface(),
			|state: &mut WaylandState, _dh, surf| {
				if surface_panel_item(surf).is_some() {
					return;
				}
				let client = surf.client().unwrap();
				let client_state = client.get_data::<ClientState>().unwrap();
				let Some(toplevel) = state
					.xdg_shell
					.toplevel_surfaces()
					.iter()
					.find(|s| s.wl_surface() == surf)
				else {
					return;
				};
				let (node, panel_item) = PanelItem::create(
					Box::new(XdgBackend::create(
						toplevel.clone(),
						client_state.seat.clone(),
					)),
					client_state.pid,
				);
				handle_cursor(&panel_item, panel_item.backend.seat.cursor_info_rx.clone());
				surf.insert_data(Arc::downgrade(&panel_item));
				surf.insert_data(node);
			},
		);

		add_post_commit_hook(
			toplevel.wl_surface(),
			|_state: &mut WaylandState, _dh, surf| {
				let Some(panel_item) = surface_panel_item(surf) else {
					return;
				};
				let Some(core_surface) = CoreSurface::from_wl_surface(surf) else {
					return;
				};
				surf.get_data_raw::<Mutex<Vector2<u32>>, _, _>(|old_size| {
					let mut old_size = old_size.lock();
					let Some(size) = core_surface.size() else {
						return;
					};
					if *old_size != size {
						panel_item.toplevel_size_changed(size);
						*old_size = size;
					}
				});
			},
		);
	}
	fn toplevel_destroyed(&mut self, toplevel: ToplevelSurface) {
		let Some(panel_item) = surface_panel_item(toplevel.wl_surface()) else {
			return;
		};
		panel_item.backend.seat.unfocus(toplevel.wl_surface(), self);
		panel_item.backend.toplevel.lock().take();
		panel_item.backend.popups.lock().clear();
	}
	fn app_id_changed(&mut self, toplevel: ToplevelSurface) {
		let Some(panel_item) = surface_panel_item(toplevel.wl_surface()) else {
			return;
		};

		panel_item.toplevel_app_id_changed(
			&toplevel
				.wl_surface()
				.get_data_raw::<XdgToplevelSurfaceData, _, _>(|d| {
					d.lock().unwrap().app_id.clone().unwrap()
				})
				.unwrap_or_default(),
		)
	}
	fn title_changed(&mut self, toplevel: ToplevelSurface) {
		let Some(panel_item) = surface_panel_item(toplevel.wl_surface()) else {
			return;
		};

		panel_item.toplevel_title_changed(
			&toplevel
				.wl_surface()
				.get_data_raw::<XdgToplevelSurfaceData, _, _>(|d| {
					d.lock().unwrap().title.clone().unwrap()
				})
				.unwrap_or_default(),
		)
	}

	fn new_popup(&mut self, popup: PopupSurface, positioner: PositionerState) {
		let uid = rand::thread_rng().gen_range(0..u64::MAX);
		popup.wl_surface().insert_data(SurfaceId::Child(uid));
		let Some(parent) = popup.get_parent_surface() else {
			return;
		};
		let _ = popup.send_configure();
		CoreSurface::add_to(popup.wl_surface());

		let Some(panel_item) = surface_panel_item(&parent) else {
			return;
		};
		let panel_item_weak = Arc::downgrade(&panel_item);
		add_post_commit_hook(
			popup.wl_surface(),
			move |state: &mut WaylandState, _dh, surf| {
				if surface_panel_item(surf).is_some() {
					return;
				}
				surf.insert_data(panel_item_weak.clone());
				let Some(panel) = surface_panel_item(surf) else {
					return;
				};
				let Some(popup) = state
					.xdg_shell
					.popup_surfaces()
					.iter()
					.find(|p| p.wl_surface() == surf)
				else {
					return;
				};
				panel.backend.new_popup(uid, popup.clone(), positioner);
			},
		);
	}
	fn reposition_request(
		&mut self,
		popup: PopupSurface,
		positioner: PositionerState,
		_token: u32,
	) {
		let Some(panel_item) = surface_panel_item(popup.wl_surface()) else {
			return;
		};
		let Some(SurfaceId::Child(uid)) = popup.wl_surface().get_data::<SurfaceId>() else {
			return;
		};

		panel_item.backend.reposition_popup(uid, popup, positioner)
	}
	fn popup_destroyed(&mut self, popup: PopupSurface) {
		let Some(panel_item) = surface_panel_item(popup.wl_surface()) else {
			return;
		};
		let Some(SurfaceId::Child(uid)) = popup.wl_surface().get_data::<SurfaceId>() else {
			return;
		};
		panel_item.backend.seat.unfocus(popup.wl_surface(), self);
		panel_item.backend.drop_popup(uid);
	}

	fn grab(&mut self, _popup: PopupSurface, _seat: WlSeat, _serial: Serial) {}

	fn move_request(&mut self, toplevel: ToplevelSurface, _seat: WlSeat, _serial: Serial) {
		let Some(panel_item) = surface_panel_item(toplevel.wl_surface()) else {
			return;
		};
		panel_item.toplevel_move_request();
	}
	fn resize_request(
		&mut self,
		toplevel: ToplevelSurface,
		_seat: WlSeat,
		_serial: Serial,
		edges: ResizeEdge,
	) {
		let Some(panel_item) = surface_panel_item(toplevel.wl_surface()) else {
			return;
		};
		let (up, down, left, right) = match edges {
			ResizeEdge::None => (false, false, false, false),
			ResizeEdge::Top => (true, false, false, false),
			ResizeEdge::Bottom => (false, true, false, false),
			ResizeEdge::Left => (false, false, true, false),
			ResizeEdge::TopLeft => (true, false, true, false),
			ResizeEdge::BottomLeft => (false, true, true, false),
			ResizeEdge::Right => (false, false, false, true),
			ResizeEdge::TopRight => (true, false, false, true),
			ResizeEdge::BottomRight => (false, true, false, true),
			_ => (false, false, false, false),
		};
		panel_item.toplevel_resize_request(up, down, left, right);
	}

	fn maximize_request(&mut self, toplevel: ToplevelSurface) {
		toplevel.with_pending_state(|s| {
			s.states.set(State::Maximized);
			s.states.unset(State::Fullscreen);
		});
		toplevel.send_configure();

		let Some(panel_item) = surface_panel_item(toplevel.wl_surface()) else {
			return;
		};
		panel_item.toplevel_fullscreen_active(false);
	}
	fn fullscreen_request(&mut self, toplevel: ToplevelSurface, _output: Option<WlOutput>) {
		toplevel.with_pending_state(|s| {
			s.states.set(State::Fullscreen);
			s.states.unset(State::Maximized);
		});
		toplevel.send_configure();

		let Some(panel_item) = surface_panel_item(toplevel.wl_surface()) else {
			return;
		};
		panel_item.toplevel_fullscreen_active(true);
	}
}
delegate_xdg_shell!(WaylandState);

pub struct XdgBackend {
	toplevel: Mutex<Option<ToplevelSurface>>,
	popups: Mutex<FxHashMap<u64, (PopupSurface, PositionerState)>>,
	pub seat: Arc<SeatWrapper>,
}
impl XdgBackend {
	pub fn create(toplevel: ToplevelSurface, seat: Arc<SeatWrapper>) -> Self {
		XdgBackend {
			toplevel: Mutex::new(Some(toplevel)),
			popups: Mutex::new(FxHashMap::default()),
			seat,
		}
	}
	fn wl_surface_from_id(&self, id: &SurfaceId) -> Option<WlSurface> {
		match id {
			SurfaceId::Toplevel(_) => Some(self.toplevel.lock().clone()?.wl_surface().clone()),
			SurfaceId::Child(popup) => {
				let popups = self.popups.lock();
				Some(popups.get(popup)?.0.wl_surface().clone())
			}
		}
	}
	fn panel_item(&self) -> Option<Arc<PanelItem<XdgBackend>>> {
		surface_panel_item(self.toplevel.lock().clone()?.wl_surface())
	}

	pub fn new_popup(&self, id: u64, popup: PopupSurface, positioner: PositionerState) {
		let Some(panel_item) = self.panel_item() else {
			return;
		};

		self.popups.lock().insert(id, (popup, positioner));

		let child_data = self.child_data(id).unwrap();
		panel_item.create_child(id, &child_data);
	}
	pub fn reposition_popup(&self, id: u64, _popup: PopupSurface, positioner: PositionerState) {
		let mut popups = self.popups.lock();
		let Some((_, old_positioner)) = popups.get_mut(&id) else {
			return;
		};
		let Some(panel_item) = self.panel_item() else {
			return;
		};
		let geometry = positioner.get_geometry();

		*old_positioner = positioner;
		panel_item.reposition_child(id, &geometry.into());
	}
	pub fn drop_popup(&self, id: u64) {
		let Some(panel_item) = self.panel_item() else {
			return;
		};
		panel_item.destroy_child(id);
	}

	fn child_data(&self, id: u64) -> Option<ChildInfo> {
		let (popup, positioner) = self.popups.lock().get(&id).unwrap().clone();
		let parent = popup.get_parent_surface().unwrap();
		let parent = parent.get_data::<SurfaceId>().unwrap();
		Some(ChildInfo {
			id,
			parent,
			geometry: positioner
				.get_unconstrained_geometry(Rectangle {
					loc: (-100000, -100000).into(),
					size: (200000, 200000).into(),
				})
				.into(),
		})
	}
}
impl Backend for XdgBackend {
	fn start_data(&self) -> Result<PanelItemInitData> {
		let cursor = self
			.seat
			.cursor_info_rx
			.borrow()
			.surface
			.clone()
			.and_then(|s| s.upgrade().ok())
			.as_ref()
			.and_then(CoreSurface::from_wl_surface)
			.and_then(|c| c.size())
			.map(|size| Geometry {
				origin: [0; 2].into(),
				size,
			});

		let toplevel = self.toplevel.lock().clone().unwrap();
		let app_id = compositor::with_states(toplevel.wl_surface(), |states| {
			states
				.data_map
				.get::<XdgToplevelSurfaceData>()
				.unwrap()
				.lock()
				.unwrap()
				.title
				.clone()
		});
		let title = compositor::with_states(toplevel.wl_surface(), |states| {
			states
				.data_map
				.get::<XdgToplevelSurfaceData>()
				.unwrap()
				.lock()
				.unwrap()
				.app_id
				.clone()
		});
		let toplevel_cached_state = compositor::with_states(toplevel.wl_surface(), |states| {
			*states.cached_state.get::<SurfaceCachedState>().current()
		});
		let toplevel_core_surface = CoreSurface::from_wl_surface(toplevel.wl_surface()).unwrap();

		let size = toplevel
			.current_state()
			.size
			.map(|s| Vector2::from([s.w as u32, s.h as u32]))
			.or_else(|| toplevel_core_surface.size())
			.unwrap_or(Vector2::from([0; 2]));
		let parent = toplevel
			.parent()
			.as_ref()
			.and_then(surface_panel_item)
			.and_then(|p| p.node.upgrade())
			.map(|p| p.get_id());
		let toplevel = ToplevelInfo {
			parent,
			title,
			app_id,
			size,
			min_size: if toplevel_cached_state.min_size.w != 0
				&& toplevel_cached_state.min_size.h != 0
			{
				Some(
					[
						toplevel_cached_state.min_size.w as f32,
						toplevel_cached_state.min_size.h as f32,
					]
					.into(),
				)
			} else {
				None
			},
			max_size: if toplevel_cached_state.max_size.w != 0
				&& toplevel_cached_state.max_size.h != 0
			{
				Some(
					[
						toplevel_cached_state.max_size.w as f32,
						toplevel_cached_state.max_size.h as f32,
					]
					.into(),
				)
			} else {
				None
			},
			logical_rectangle: toplevel_cached_state
				.geometry
				.map(Into::into)
				.unwrap_or_else(|| Geometry {
					origin: [0; 2].into(),
					size,
				}),
		};

		let children = self
			.popups
			.lock()
			.keys()
			.map(|k| self.child_data(*k).unwrap())
			.collect();

		Ok(PanelItemInitData {
			cursor,
			toplevel,
			children,
			pointer_grab: None,
			keyboard_grab: None,
		})
	}
	fn apply_cursor_material(&self, model_part: &Arc<ModelPart>) {
		let Some(surface) = self
			.seat
			.cursor_info_rx
			.borrow()
			.surface
			.clone()
			.and_then(|s| s.upgrade().ok())
		else {
			return;
		};

		let Some(core_surface) = CoreSurface::from_wl_surface(&surface) else {
			return;
		};
		core_surface.apply_material(model_part);
	}
	fn apply_surface_material(&self, surface: SurfaceId, model_part: &Arc<ModelPart>) {
		let Some(surface) = self.wl_surface_from_id(&surface) else {
			return;
		};
		let Some(core_surface) = CoreSurface::from_wl_surface(&surface) else {
			return;
		};
		core_surface.apply_material(model_part);
	}

	fn close_toplevel(&self) {
		if let Some(toplevel) = self.toplevel.lock().clone() {
			toplevel.send_close();
		}
	}

	fn auto_size_toplevel(&self) {
		let Some(toplevel) = self.toplevel.lock().clone() else {
			return;
		};
		toplevel.with_pending_state(|s| s.size = None);
		toplevel.send_configure();
	}
	fn set_toplevel_size(&self, size: Vector2<u32>) {
		let Some(toplevel) = self.toplevel.lock().clone() else {
			return;
		};
		toplevel.with_pending_state(|s| {
			s.size = Some((size.x.max(16) as i32, size.y.max(16) as i32).into())
		});
		toplevel.send_pending_configure();
	}
	fn set_toplevel_focused_visuals(&self, focused: bool) {
		let Some(toplevel) = self.toplevel.lock().clone() else {
			return;
		};
		toplevel.with_pending_state(|s| {
			if focused {
				s.states.set(State::Activated);
			} else {
				s.states.unset(State::Activated);
			}
		})
	}

	fn pointer_motion(&self, surface: &SurfaceId, position: Vector2<f32>) {
		let Some(surface) = self.wl_surface_from_id(surface) else {
			return;
		};
		self.seat.pointer_motion(surface, position)
	}
	fn pointer_button(&self, _surface: &SurfaceId, button: u32, pressed: bool) {
		self.seat.pointer_button(button, pressed)
	}
	fn pointer_scroll(
		&self,
		_surface: &SurfaceId,
		scroll_distance: Option<Vector2<f32>>,
		scroll_steps: Option<Vector2<f32>>,
	) {
		self.seat.pointer_scroll(scroll_distance, scroll_steps)
	}

	fn keyboard_keys(&self, surface: &SurfaceId, keymap_id: u64, keys: Vec<i32>) {
		let Some(surface) = self.wl_surface_from_id(surface) else {
			return;
		};
		self.seat.keyboard_keys(surface, keymap_id, keys)
	}

	fn touch_down(&self, surface: &SurfaceId, id: u32, position: Vector2<f32>) {
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
	fn reset_input(&self) {
		self.seat.reset_input()
	}
}

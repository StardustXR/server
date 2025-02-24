use super::{
	seat::{SeatWrapper, handle_cursor},
	state::{ClientState, WaylandState},
	surface::CoreSurface,
	utils::*,
};
use crate::{
	core::error::{Result, ServerError},
	nodes::{
		drawable::model::ModelPart,
		items::panel::{
			Backend, ChildInfo, Geometry, PanelItem, PanelItemInitData, SurfaceId, ToplevelInfo,
		},
	},
};
use color_eyre::eyre::eyre;
use mint::Vector2;
use parking_lot::Mutex;
use rand::Rng;
use rustc_hash::FxHashMap;
use smithay::{
	delegate_xdg_shell,
	desktop::PopupKind,
	reexports::{
		wayland_protocols::xdg::{
			decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode,
			shell::server::xdg_toplevel::{ResizeEdge, State},
		},
		wayland_server::{
			Resource,
			protocol::{wl_output::WlOutput, wl_seat::WlSeat, wl_surface::WlSurface},
		},
	},
	utils::{Logical, Rectangle, Serial},
	wayland::{
		compositor::add_post_commit_hook,
		shell::xdg::{
			PopupSurface, PositionerState, ShellClient, ToplevelSurface, XdgShellHandler,
			XdgShellState,
		},
	},
};
use std::sync::{Arc, Weak};
use tracing::warn;

fn get_unconstrained_popup_geometry(positioner: &PositionerState) -> Geometry {
	positioner
		.get_unconstrained_geometry(Rectangle {
			loc: (-100000, -100000).into(),
			size: (200000, 200000).into(),
		})
		.into()
}

impl From<Rectangle<i32, Logical>> for Geometry {
	fn from(value: Rectangle<i32, Logical>) -> Self {
		Geometry {
			origin: [value.loc.x, value.loc.y].into(),
			size: [value.size.w as u32, value.size.h as u32].into(),
		}
	}
}

pub fn surface_panel_item(wl_surface: &WlSurface) -> Option<Arc<PanelItem<XdgBackend>>> {
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

		let initial_size = toplevel
			.wl_surface()
			.get_size()
			.unwrap_or(Vector2::from([0; 2]));

		let initial_toplevel_info = ToplevelInfo {
			parent: toplevel.wl_surface().get_parent(),
			title: toplevel.wl_surface().get_title(),
			app_id: toplevel.wl_surface().get_app_id(),
			size: initial_size,
			min_size: toplevel
				.wl_surface()
				.min_size()
				.map(|s| Vector2::from([s.x as f32, s.y as f32])),
			max_size: toplevel
				.wl_surface()
				.max_size()
				.map(|s| Vector2::from([s.x as f32, s.y as f32])),
			logical_rectangle: toplevel.wl_surface().get_geometry().unwrap_or(Geometry {
				origin: [0; 2].into(),
				size: initial_size,
			}),
		};
		toplevel
			.wl_surface()
			.insert_data(Mutex::new(initial_toplevel_info));

		CoreSurface::add_to(toplevel.wl_surface());

		add_post_commit_hook(
			toplevel.wl_surface(),
			|_state: &mut WaylandState, _dh, surf| {
				let parent = surf.get_parent();
				let new_size = surf.get_size().unwrap_or(Vector2::from([0; 2]));
				let min_size = surf
					.min_size()
					.map(|s| Vector2::from([s.x as f32, s.y as f32]));
				let max_size = surf
					.max_size()
					.map(|s| Vector2::from([s.x as f32, s.y as f32]));
				let logical_rectangle = surf.get_geometry().unwrap_or_default();

				let mut size_changed = false;
				surf.with_toplevel_info(|info| {
					info.parent = parent;
					if new_size != info.size {
						info.size = new_size;
						size_changed = true;
					}
					info.min_size = min_size;
					info.max_size = max_size;
					info.logical_rectangle = logical_rectangle;
				});

				if size_changed {
					let Some(panel_item) = surface_panel_item(surf) else {
						return;
					};
					if let Some(toplevel_info) = surf.get_toplevel_info() {
						panel_item.toplevel_size_changed(toplevel_info.size);
					}
				}
			},
		);

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
	}
	fn toplevel_destroyed(&mut self, toplevel: ToplevelSurface) {
		let Some(panel_item) = surface_panel_item(toplevel.wl_surface()) else {
			return;
		};
		panel_item.backend.seat.unfocus(toplevel.wl_surface(), self);
		panel_item.backend.toplevel.lock().take();
		panel_item.backend.children.lock().clear();
	}
	fn app_id_changed(&mut self, toplevel: ToplevelSurface) {
		let wl_surface = toplevel.wl_surface();
		let Some(app_id) = wl_surface.get_app_id() else {
			return;
		};

		wl_surface.with_toplevel_info(|info| {
			info.app_id = Some(app_id.clone());
		});

		let Some(panel_item) = surface_panel_item(wl_surface) else {
			return;
		};
		panel_item.toplevel_app_id_changed(&app_id)
	}

	fn title_changed(&mut self, toplevel: ToplevelSurface) {
		let wl_surface = toplevel.wl_surface();
		let Some(title) = wl_surface.get_title() else {
			return;
		};

		wl_surface.with_toplevel_info(|info| {
			info.title = Some(title.clone());
		});

		let Some(panel_item) = surface_panel_item(wl_surface) else {
			return;
		};
		panel_item.toplevel_title_changed(&title)
	}
	fn new_popup(&mut self, popup: PopupSurface, positioner: PositionerState) {
		self.popup_manager
			.track_popup(PopupKind::Xdg(popup.clone()))
			.unwrap();

		let id = rand::thread_rng().gen_range(0..u64::MAX);
		popup.wl_surface().insert_data(SurfaceId::Child(id));
		let Some(parent) = popup.get_parent_surface() else {
			warn!("No parent surface found for popup");
			return;
		};
		CoreSurface::add_to(popup.wl_surface());

		popup.wl_surface().insert_data(Mutex::new(ChildInfo {
			id,
			parent: parent.get_data::<SurfaceId>().unwrap(),
			geometry: get_unconstrained_popup_geometry(&positioner),
			z_order: 1,
			receives_input: true,
		}));

		let Some(panel_item) = surface_panel_item(&parent) else {
			warn!("No panel item found for popup parent");
			return;
		};
		let panel_item_weak = Arc::downgrade(&panel_item);
		add_post_commit_hook(
			popup.wl_surface(),
			move |_: &mut WaylandState, _dh, surf| {
				if surface_panel_item(surf).is_some() {
					return;
				}
				surf.insert_data(panel_item_weak.clone());
				let Some(panel) = surface_panel_item(surf) else {
					warn!("Failed to get panel item for popup surface");
					return;
				};
				panel.backend.new_child(surf);
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

		popup.wl_surface().with_child_info(|ci| {
			ci.geometry = get_unconstrained_popup_geometry(&positioner);
		});

		panel_item.backend.reposition_child(popup.wl_surface());
	}
	fn popup_destroyed(&mut self, popup: PopupSurface) {
		let Some(panel_item) = surface_panel_item(popup.wl_surface()) else {
			return;
		};
		panel_item.backend.seat.unfocus(popup.wl_surface(), self);
		panel_item.backend.drop_child(popup.wl_surface());
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
	pub children: Mutex<FxHashMap<u64, WlSurface>>,
	seat: Arc<SeatWrapper>,
}
impl XdgBackend {
	pub fn create(toplevel: ToplevelSurface, seat: Arc<SeatWrapper>) -> Self {
		XdgBackend {
			toplevel: Mutex::new(Some(toplevel)),
			children: Mutex::new(FxHashMap::default()),
			seat,
		}
	}
	fn wl_surface_from_id(&self, id: &SurfaceId) -> Option<WlSurface> {
		match id {
			SurfaceId::Toplevel(_) => Some(self.toplevel.lock().clone()?.wl_surface().clone()),
			SurfaceId::Child(id) => self.children.lock().get(id).cloned(),
		}
	}
	fn panel_item(&self) -> Option<Arc<PanelItem<XdgBackend>>> {
		surface_panel_item(self.toplevel.lock().clone()?.wl_surface())
	}

	pub fn new_child(&self, surface: &WlSurface) {
		let Some(panel_item) = self.panel_item() else {
			return;
		};
		let Some(child_info) = surface.get_child_info() else {
			return;
		};

		self.children.lock().insert(child_info.id, surface.clone());
		panel_item.create_child(child_info.id, &child_info);
	}
	pub fn reposition_child(&self, surface: &WlSurface) {
		let Some(panel_item) = self.panel_item() else {
			return;
		};
		let Some(child_info) = surface.get_child_info() else {
			return;
		};

		panel_item.reposition_child(child_info.id, &child_info.geometry);
	}
	pub fn drop_child(&self, surface: &WlSurface) {
		let Some(panel_item) = self.panel_item() else {
			return;
		};
		let Some(child_info) = surface.get_child_info() else {
			return;
		};
		panel_item.destroy_child(child_info.id);
		self.children.lock().remove(&child_info.id);
	}
}
impl Backend for XdgBackend {
	fn start_data(&self) -> Result<PanelItemInitData, ServerError> {
		let cursor = self
			.seat
			.cursor_info_rx
			.borrow()
			.surface
			.clone()
			.and_then(|s| s.upgrade().ok())
			.as_ref()
			.and_then(|c| c.get_size())
			.map(|size| Geometry {
				origin: [0; 2].into(),
				size,
			});

		let toplevel_info = self
			.toplevel
			.lock()
			.as_ref()
			.and_then(|toplevel| toplevel.wl_surface().get_toplevel_info())
			.ok_or_else(|| ServerError::Report(eyre!("Internal: no toplevel or ToplevelInfo")))?;

		let children = self
			.children
			.lock()
			.values()
			.filter_map(|v| v.get_child_info())
			.collect();

		Ok(PanelItemInitData {
			cursor,
			toplevel: toplevel_info,
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

	fn keyboard_key(&self, surface: &SurfaceId, keymap_id: u64, key: u32, pressed: bool) {
		let Some(surface) = self.wl_surface_from_id(surface) else {
			return;
		};
		self.seat.keyboard_key(surface, keymap_id, key, pressed)
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

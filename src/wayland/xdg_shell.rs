use std::sync::Arc;

use super::{
	panel_item::{PanelItem, RecommendedState, ToplevelState},
	state::WaylandState,
	surface::{CoreSurface, SurfaceGeometry},
	SERIAL_COUNTER,
};
use mint::Vector2;
use parking_lot::Mutex;
use serde::Serialize;
use smithay::{
	reexports::{
		wayland_protocols::xdg::shell::server::{
			xdg_popup::{self, XdgPopup},
			xdg_positioner::{self, Anchor, ConstraintAdjustment, Gravity, XdgPositioner},
			xdg_surface::{self, XdgSurface},
			xdg_toplevel::{self, XdgToplevel, EVT_WM_CAPABILITIES_SINCE},
			xdg_wm_base::{self, XdgWmBase},
		},
		wayland_server::{
			protocol::wl_surface::WlSurface, Client, DataInit, Dispatch, DisplayHandle,
			GlobalDispatch, New, Resource, WEnum, Weak,
		},
	},
	wayland::compositor,
};

// impl XdgShellHandler for WaylandState {
// 	fn xdg_shell_state(&mut self) -> &mut WaylandState {
// 		&mut self.xdg_shell_state
// 	}

// 	fn new_toplevel(&mut self, surface: ToplevelSurface) {
// 		self.output.enter(surface.wl_surface());
// 		surface.with_pending_state(|state| {
// 			state.states.set(State::Maximized);
// 			state.states.set(State::Activated);
// 			state.decoration_mode = Some(Mode::ServerSide);
// 		});
// 		surface.send_configure();

// 		let client = surface.wl_surface().client().unwrap();
// 		let (node, item) = PanelItem::create(
// 			&surface,
// 			client.get_credentials(&self.display_handle).ok(),
// 			self.seats.get(&client.id()).unwrap().clone(),
// 		);
// 		compositor::with_states(surface.wl_surface(), |surface_data| {
// 			surface_data.data_map.insert_if_missing_threadsafe(|| node);
// 			surface_data.data_map.insert_if_missing_threadsafe(|| item);
// 		});
// 	}
// 	fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
// 		self.output.enter(surface.wl_surface());
// 		let _ = surface.send_configure();
// 		// let panel_item = compositor::with_states(&surface.get_parent_surface().unwrap(), |data| {
// 		// 	data.data_map.get::<Arc<PanelItem>>().cloned()
// 		// });
// 	}
// 	fn ack_configure(&mut self, surface: WlSurface, configure: Configure) {
// 		compositor::with_states(&surface, |data| {
// 			if let Some(panel_item) = data.data_map.get::<Arc<PanelItem>>() {
// 				panel_item.ack_resize(configure);
// 			}
// 		});
// 	}

// 	fn grab(&mut self, _surface: PopupSurface, _seat: WlSeat, _serial: Serial) {}
// }
// delegate_xdg_shell!(WaylandState);

impl GlobalDispatch<XdgWmBase, (), WaylandState> for WaylandState {
	fn bind(
		_state: &mut WaylandState,
		_handle: &DisplayHandle,
		_client: &Client,
		resource: New<XdgWmBase>,
		_global_data: &(),
		data_init: &mut DataInit<'_, WaylandState>,
	) {
		data_init.init(resource, ());
	}
}
#[derive(Debug)]
pub struct WaylandSurface {
	wl_surface: Weak<WlSurface>,
	geometry: Arc<Mutex<Option<SurfaceGeometry>>>,
}

impl Dispatch<XdgWmBase, (), WaylandState> for WaylandState {
	fn request(
		_state: &mut WaylandState,
		_client: &Client,
		_resource: &XdgWmBase,
		request: xdg_wm_base::Request,
		_data: &(),
		_dhandle: &DisplayHandle,
		data_init: &mut DataInit<'_, WaylandState>,
	) {
		match request {
			xdg_wm_base::Request::CreatePositioner { id } => {
				data_init.init(id, Arc::new(Mutex::new(PositionerData::default())));
			}
			xdg_wm_base::Request::GetXdgSurface { id, surface } => {
				data_init.init(
					id,
					WaylandSurface {
						wl_surface: surface.downgrade(),
						geometry: Arc::new(Mutex::new(None)),
					},
				);
			}
			xdg_wm_base::Request::Pong { serial: _ } => (),
			xdg_wm_base::Request::Destroy => (),
			_ => unreachable!(),
		}
	}
}

#[derive(Debug, Serialize)]
pub struct PositionerData {
	size: Vector2<u32>,
	anchor_rect_pos: Vector2<i32>,
	anchor_rect_size: Vector2<u32>,
	anchor: u32,
	gravity: u32,
	constraint_adjustment: u32,
	offset: Vector2<i32>,
	reactive: bool,
}
impl Default for PositionerData {
	fn default() -> Self {
		Self {
			size: Vector2::from([0; 2]),
			anchor_rect_pos: Vector2::from([0; 2]),
			anchor_rect_size: Vector2::from([0; 2]),
			anchor: Anchor::None as u32,
			gravity: Gravity::None as u32,
			constraint_adjustment: ConstraintAdjustment::None.bits(),
			offset: Vector2::from([0; 2]),
			reactive: false,
		}
	}
}

impl Dispatch<XdgPositioner, Arc<Mutex<PositionerData>>, WaylandState> for WaylandState {
	fn request(
		_state: &mut WaylandState,
		_client: &Client,
		resource: &XdgPositioner,
		request: xdg_positioner::Request,
		data: &Arc<Mutex<PositionerData>>,
		_dhandle: &DisplayHandle,
		_data_init: &mut DataInit<'_, WaylandState>,
	) {
		match request {
			xdg_positioner::Request::SetSize { width, height } => {
				data.lock().size = Vector2::from([width as u32, height as u32]);
			}
			xdg_positioner::Request::SetAnchorRect {
				x,
				y,
				width,
				height,
			} => {
				if width < 1 || height < 1 {
					resource.post_error(
						xdg_positioner::Error::InvalidInput,
						"Invalid size for positioner's anchor rectangle.",
					);
					return;
				}

				let mut data = data.lock();
				data.anchor_rect_pos = [x, y].into();
				data.anchor_rect_size = [width as u32, height as u32].into();
			}
			xdg_positioner::Request::SetAnchor { anchor } => {
				if let WEnum::Value(anchor) = anchor {
					data.lock().anchor = anchor as u32;
				}
			}
			xdg_positioner::Request::SetGravity { gravity } => {
				if let WEnum::Value(gravity) = gravity {
					data.lock().gravity = gravity as u32;
				}
			}
			xdg_positioner::Request::SetConstraintAdjustment {
				constraint_adjustment,
			} => {
				data.lock().constraint_adjustment = constraint_adjustment;
			}
			xdg_positioner::Request::SetOffset { x, y } => {
				data.lock().offset = [x, y].into();
			}
			xdg_positioner::Request::SetReactive => {
				data.lock().reactive = true;
			}
			xdg_positioner::Request::SetParentSize {
				parent_width: _,
				parent_height: _,
			} => (),
			xdg_positioner::Request::SetParentConfigure { serial: _ } => (),
			xdg_positioner::Request::Destroy => (),
			_ => unreachable!(),
		}
	}
}

#[derive(Debug, Clone)]
pub struct XdgSurfaceData {
	pub wl_surface: Weak<WlSurface>,
	pub xdg_surface: Weak<XdgSurface>,
	pub geometry: Arc<Mutex<Option<SurfaceGeometry>>>,
}
impl Dispatch<XdgSurface, WaylandSurface, WaylandState> for WaylandState {
	fn request(
		state: &mut WaylandState,
		client: &Client,
		xdg_surface: &XdgSurface,
		request: xdg_surface::Request,
		data: &WaylandSurface,
		_dhandle: &DisplayHandle,
		data_init: &mut DataInit<'_, WaylandState>,
	) {
		match request {
			xdg_surface::Request::GetToplevel { id } => {
				let toplevel_state = Arc::new(Mutex::new(ToplevelState {
					queued_state: Some(Box::new(ToplevelState::default())),
					..Default::default()
				}));
				let toplevel = data_init.init(
					id,
					XdgToplevelData {
						state: toplevel_state,
						xdg_surface_data: XdgSurfaceData {
							wl_surface: data.wl_surface.clone(),
							xdg_surface: xdg_surface.downgrade(),
							geometry: data.geometry.clone(),
						},
					},
				);

				if toplevel.version() >= EVT_WM_CAPABILITIES_SINCE {
					toplevel.wm_capabilities(vec![]);
				}
				toplevel.configure(0, 0, vec![]);
				xdg_surface.configure(SERIAL_COUNTER.inc());

				let (node, item) = PanelItem::create(
					toplevel,
					client.get_credentials(&state.display_handle).ok(),
					state.seats.get(&client.id()).unwrap().clone(),
				);
				compositor::with_states(&data.wl_surface.upgrade().unwrap(), |surface_data| {
					surface_data.data_map.insert_if_missing_threadsafe(|| node);
					surface_data.data_map.insert_if_missing_threadsafe(|| item);
				});
			}
			xdg_surface::Request::GetPopup {
				id,
				parent: _,
				positioner: _,
			} => {
				data_init.init(id, ());
			}
			xdg_surface::Request::SetWindowGeometry {
				x,
				y,
				width,
				height,
			} => {
				let geometry = SurfaceGeometry {
					origin: [x as u32, y as u32].into(),
					size: [width as u32, height as u32].into(),
				};
				*data.geometry.lock() = Some(geometry);
				let Ok(wl_surface) = data.wl_surface.upgrade() else { return; };
				compositor::with_states(&wl_surface, |data| {
					if let Some(core_surface) = data.data_map.get::<CoreSurface>() {
						core_surface.set_geometry(geometry);
					}
				});
			}
			xdg_surface::Request::AckConfigure { serial: _ } => (),
			xdg_surface::Request::Destroy => (),
			_ => unreachable!(),
		}
	}
}

#[derive(Debug)]
pub struct XdgToplevelData {
	pub state: Arc<Mutex<ToplevelState>>,
	pub xdg_surface_data: XdgSurfaceData,
}
impl XdgToplevelData {
	fn panel_item(&self) -> Option<Arc<PanelItem>> {
		let wl_surface = self.xdg_surface_data.wl_surface.upgrade().ok()?;
		compositor::with_states(&wl_surface, |data| {
			data.data_map.get::<Arc<PanelItem>>().cloned()
		})
	}
}
impl Dispatch<XdgToplevel, XdgToplevelData, WaylandState> for WaylandState {
	fn request(
		_state: &mut WaylandState,
		_client: &Client,
		_resource: &XdgToplevel,
		request: xdg_toplevel::Request,
		data: &XdgToplevelData,
		_dhandle: &DisplayHandle,
		_data_init: &mut DataInit<'_, WaylandState>,
	) {
		match request {
			xdg_toplevel::Request::SetParent { parent } => {
				let mut state = data.state.lock();
				let queued_state = state.queued_state.as_mut().unwrap();
				queued_state.parent = parent.map(|toplevel| toplevel.downgrade());
			}
			xdg_toplevel::Request::SetTitle { title } => {
				let mut state = data.state.lock();
				let queued_state = state.queued_state.as_mut().unwrap();
				queued_state.title = (!title.is_empty()).then_some(title);
			}
			xdg_toplevel::Request::SetAppId { app_id } => {
				let mut state = data.state.lock();
				let queued_state = state.queued_state.as_mut().unwrap();
				queued_state.app_id = (!app_id.is_empty()).then_some(app_id);
			}
			xdg_toplevel::Request::ShowWindowMenu {
				seat: _,
				serial: _,
				x: _,
				y: _,
			} => (),
			xdg_toplevel::Request::Move { seat: _, serial: _ } => {
				let Some(panel_item) = data.panel_item() else { return };
				panel_item.recommend_toplevel_state(RecommendedState::Move);
			}
			xdg_toplevel::Request::Resize {
				seat: _,
				serial: _,
				edges,
			} => {
				let WEnum::Value(edges) = edges else { return };
				let Some(panel_item) = data.panel_item() else { return };
				panel_item.recommend_toplevel_state(RecommendedState::Resize(edges as u32));
			}
			xdg_toplevel::Request::SetMaxSize { width, height } => {
				let mut state = data.state.lock();
				let queued_state = state.queued_state.as_mut().unwrap();
				queued_state.max_size = (width > 1 || height > 1)
					.then_some(Vector2::from([width as u32, height as u32]));
			}
			xdg_toplevel::Request::SetMinSize { width, height } => {
				let mut state = data.state.lock();
				let queued_state = state.queued_state.as_mut().unwrap();
				queued_state.min_size = (width > 1 || height > 1)
					.then_some(Vector2::from([width as u32, height as u32]));
			}
			xdg_toplevel::Request::SetMaximized => {
				let Some(panel_item) = data.panel_item() else { return };
				panel_item.recommend_toplevel_state(RecommendedState::Maximize(true));
			}
			xdg_toplevel::Request::UnsetMaximized => {
				let Some(panel_item) = data.panel_item() else { return };
				panel_item.recommend_toplevel_state(RecommendedState::Maximize(false));
			}
			xdg_toplevel::Request::SetFullscreen { output: _ } => {
				let Some(panel_item) = data.panel_item() else { return };
				panel_item.recommend_toplevel_state(RecommendedState::Fullscreen(true));
			}
			xdg_toplevel::Request::UnsetFullscreen => {
				let Some(panel_item) = data.panel_item() else { return };
				panel_item.recommend_toplevel_state(RecommendedState::Fullscreen(true));
			}
			xdg_toplevel::Request::SetMinimized => {
				let Some(panel_item) = data.panel_item() else { return };
				panel_item.recommend_toplevel_state(RecommendedState::Minimize);
			}
			xdg_toplevel::Request::Destroy => {
				let Some(panel_item) = data.panel_item() else { return };
				panel_item.on_drop();
			}
			_ => unreachable!(),
		}
	}
}

impl Dispatch<XdgPopup, (), WaylandState> for WaylandState {
	fn request(
		_state: &mut WaylandState,
		_client: &Client,
		_resource: &XdgPopup,
		request: xdg_popup::Request,
		_data: &(),
		_dhandle: &DisplayHandle,
		_data_init: &mut DataInit<'_, WaylandState>,
	) {
		match request {
			xdg_popup::Request::Grab { seat: _, serial: _ } => (),
			xdg_popup::Request::Reposition {
				positioner: _,
				token: _,
			} => (),
			xdg_popup::Request::Destroy => (),
			_ => unreachable!(),
		}
	}
}

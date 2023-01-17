use super::{
	panel_item::{PanelItem, RecommendedState, ToplevelState},
	state::WaylandState,
	surface::SurfaceGeometry,
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
use std::sync::Arc;
use tracing::{debug, warn};

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
				let positioner =
					data_init.init(id, Arc::new(Mutex::new(PositionerData::default())));
				debug!(?positioner, "Create XDG positioner");
			}
			xdg_wm_base::Request::GetXdgSurface { id, surface } => {
				let xdg_surface = data_init.init(
					id,
					WaylandSurface {
						wl_surface: surface.downgrade(),
						geometry: Arc::new(Mutex::new(None)),
					},
				);
				debug!(?xdg_surface, "Create XDG surface");
			}
			xdg_wm_base::Request::Pong { serial } => {
				debug!(serial, "Client pong");
			}
			xdg_wm_base::Request::Destroy => {
				debug!("Destroy XDG WM base");
			}
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
		positioner: &XdgPositioner,
		request: xdg_positioner::Request,
		data: &Arc<Mutex<PositionerData>>,
		_dhandle: &DisplayHandle,
		_data_init: &mut DataInit<'_, WaylandState>,
	) {
		match request {
			xdg_positioner::Request::SetSize { width, height } => {
				debug!(?positioner, width, height, "Set positioner size");
				data.lock().size = Vector2::from([width as u32, height as u32]);
			}
			xdg_positioner::Request::SetAnchorRect {
				x,
				y,
				width,
				height,
			} => {
				if width < 1 || height < 1 {
					positioner.post_error(
						xdg_positioner::Error::InvalidInput,
						"Invalid size for positioner's anchor rectangle.",
					);
					warn!(
						?positioner,
						width, height, "Invalid size for positioner's anchor rectangle"
					);
					return;
				}

				debug!(
					?positioner,
					x, y, width, height, "Set positioner anchor rectangle"
				);
				let mut data = data.lock();
				data.anchor_rect_pos = [x, y].into();
				data.anchor_rect_size = [width as u32, height as u32].into();
			}
			xdg_positioner::Request::SetAnchor { anchor } => {
				if let WEnum::Value(anchor) = anchor {
					debug!(?positioner, ?anchor, "Set positioner anchor");
					data.lock().anchor = anchor as u32;
				}
			}
			xdg_positioner::Request::SetGravity { gravity } => {
				if let WEnum::Value(gravity) = gravity {
					debug!(?positioner, ?gravity, "Set positioner gravity");
					data.lock().gravity = gravity as u32;
				}
			}
			xdg_positioner::Request::SetConstraintAdjustment {
				constraint_adjustment,
			} => {
				debug!(
					?positioner,
					constraint_adjustment, "Set positioner constraint adjustment"
				);
				data.lock().constraint_adjustment = constraint_adjustment;
			}
			xdg_positioner::Request::SetOffset { x, y } => {
				debug!(?positioner, x, y, "Set positioner offset");
				data.lock().offset = [x, y].into();
			}
			xdg_positioner::Request::SetReactive => {
				debug!(?positioner, "Set positioner reactive");
				data.lock().reactive = true;
			}
			xdg_positioner::Request::SetParentSize {
				parent_width,
				parent_height,
			} => {
				debug!(
					?positioner,
					parent_width, parent_height, "Set positioner parent size"
				);
			}
			xdg_positioner::Request::SetParentConfigure { serial } => {
				debug!(?positioner, serial, "Set positioner parent size");
			}
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
				debug!(?toplevel, ?xdg_surface, "Create XDG toplevel");

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
				let popup = data_init.init(id, ());
				debug!(?popup, ?xdg_surface, "Create XDG popup");
				popup.popup_done(); // temporary hack to avoid apps locking up before popups are implemented
			}
			xdg_surface::Request::SetWindowGeometry {
				x,
				y,
				width,
				height,
			} => {
				debug!(
					?xdg_surface,
					x, y, width, height, "Set XDG surface geometry"
				);
				let geometry = SurfaceGeometry {
					origin: [x as u32, y as u32].into(),
					size: [width as u32, height as u32].into(),
				};
				*data.geometry.lock() = Some(geometry);
				let Ok(wl_surface) = data.wl_surface.upgrade() else { return; };
				compositor::with_states(&wl_surface, |data| {
					// if let Some(core_surface) = data.data_map.get::<Arc<CoreSurface>>() {
					// 	core_surface.set_geometry(geometry);
					// }
					data.data_map.insert_if_missing_threadsafe(|| geometry);
				});
			}
			xdg_surface::Request::AckConfigure { serial } => {
				debug!(?xdg_surface, serial, "Acknowledge XDG surface configure");
			}
			xdg_surface::Request::Destroy => {
				debug!(?xdg_surface, "Destroy XDG surface");
			}
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
		xdg_toplevel: &XdgToplevel,
		request: xdg_toplevel::Request,
		data: &XdgToplevelData,
		_dhandle: &DisplayHandle,
		_data_init: &mut DataInit<'_, WaylandState>,
	) {
		match request {
			xdg_toplevel::Request::SetParent { parent } => {
				debug!(?xdg_toplevel, ?parent, "Set XDG Toplevel parent");
				let mut state = data.state.lock();
				let queued_state = state.queued_state.as_mut().unwrap();
				queued_state.parent = parent.map(|toplevel| toplevel.downgrade());
			}
			xdg_toplevel::Request::SetTitle { title } => {
				debug!(?xdg_toplevel, ?title, "Set XDG Toplevel title");
				let mut state = data.state.lock();
				let queued_state = state.queued_state.as_mut().unwrap();
				queued_state.title = (!title.is_empty()).then_some(title);
			}
			xdg_toplevel::Request::SetAppId { app_id } => {
				debug!(?xdg_toplevel, ?app_id, "Set XDG Toplevel app ID");
				let mut state = data.state.lock();
				let queued_state = state.queued_state.as_mut().unwrap();
				queued_state.app_id = (!app_id.is_empty()).then_some(app_id);
			}
			xdg_toplevel::Request::ShowWindowMenu { seat, serial, x, y } => {
				debug!(
					?xdg_toplevel,
					?seat,
					serial,
					x,
					y,
					"Show XDG Toplevel window menu"
				);
			}
			xdg_toplevel::Request::Move { seat, serial } => {
				debug!(?xdg_toplevel, ?seat, serial, "XDG Toplevel move request");
				let Some(panel_item) = data.panel_item() else { return };
				panel_item.recommend_toplevel_state(RecommendedState::Move);
			}
			xdg_toplevel::Request::Resize {
				seat,
				serial,
				edges,
			} => {
				let WEnum::Value(edges) = edges else { return };
				debug!(
					?xdg_toplevel,
					?seat,
					serial,
					?edges,
					"XDG Toplevel resize request"
				);
				let Some(panel_item) = data.panel_item() else { return };
				panel_item.recommend_toplevel_state(RecommendedState::Resize(edges as u32));
			}
			xdg_toplevel::Request::SetMaxSize { width, height } => {
				debug!(?xdg_toplevel, width, height, "Set XDG Toplevel max size");
				let mut state = data.state.lock();
				let queued_state = state.queued_state.as_mut().unwrap();
				queued_state.max_size = (width > 1 || height > 1)
					.then_some(Vector2::from([width as u32, height as u32]));
			}
			xdg_toplevel::Request::SetMinSize { width, height } => {
				debug!(?xdg_toplevel, width, height, "Set XDG Toplevel min size");
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
				debug!(?xdg_toplevel, "Destroy XDG Toplevel");
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
		xdg_popup: &XdgPopup,
		request: xdg_popup::Request,
		_data: &(),
		_dhandle: &DisplayHandle,
		_data_init: &mut DataInit<'_, WaylandState>,
	) {
		match request {
			xdg_popup::Request::Grab { seat, serial } => {
				debug!(?xdg_popup, ?seat, serial, "XDG popup grab");
				xdg_popup.popup_done(); // temporary hack to avoid apps locking up before popups are implemented
			}
			xdg_popup::Request::Reposition { positioner, token } => {
				debug!(?xdg_popup, ?positioner, token, "XDG popup reposition");
				xdg_popup.popup_done(); // temporary hack to avoid apps locking up before popups are implemented
			}
			xdg_popup::Request::Destroy => {
				debug!(?xdg_popup, "Destroy XDG popup");
			}
			_ => unreachable!(),
		}
	}
}

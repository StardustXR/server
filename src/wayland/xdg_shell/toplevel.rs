use super::{backend::XdgBackend, surface::XdgSurfaceData};
use crate::{
	nodes::items::panel::{Geometry, PanelItem, ToplevelInfo},
	wayland::{
		state::WaylandState,
		surface::CoreSurface,
		utils::{self, get_data},
	},
};
use mint::Vector2;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use smithay::reexports::{
	wayland_protocols::xdg::shell::server::xdg_toplevel::{self, ResizeEdge, XdgToplevel},
	wayland_server::{
		protocol::wl_surface::WlSurface, Client, DataInit, Dispatch, DisplayHandle, Resource,
		Weak as WlWeak,
	},
};
use std::sync::Weak;
use tracing::{debug, error};
use wayland_backend::protocol::WEnum;

pub struct ToplevelData {
	panel_item: OnceCell<Weak<PanelItem<XdgBackend>>>,
	wl_surface: WlWeak<WlSurface>,
	parent: Mutex<Option<WlWeak<WlSurface>>>,
	title: Mutex<Option<String>>,
	app_id: Mutex<Option<String>>,
	max_size: Mutex<Option<Vector2<u32>>>,
	min_size: Mutex<Option<Vector2<u32>>>,
}
impl ToplevelData {
	pub fn new(wl_surface: &WlSurface) -> Self {
		ToplevelData {
			panel_item: OnceCell::new(),
			wl_surface: wl_surface.downgrade(),
			parent: Mutex::new(None),
			title: Mutex::new(None),
			app_id: Mutex::new(None),
			max_size: Mutex::new(None),
			min_size: Mutex::new(None),
		}
	}
	pub fn parent(&self) -> Option<WlSurface> {
		self.parent
			.lock()
			.as_ref()
			.map(WlWeak::upgrade)
			.map(Result::ok)
			.flatten()
	}
}
impl From<&ToplevelData> for ToplevelInfo {
	fn from(value: &ToplevelData) -> Self {
		let wl_surface = value.wl_surface.upgrade().ok();
		let size = CoreSurface::from_wl_surface(wl_surface.as_ref().unwrap())
			.unwrap()
			.size()
			.unwrap();
		let logical_rectangle = wl_surface
			.as_ref()
			.and_then(utils::get_data::<XdgSurfaceData>)
			.and_then(|d| d.geometry.lock().clone())
			.unwrap_or_else(|| Geometry {
				origin: [0, 0].into(),
				size,
			});
		let parent = value
			.parent()
			.as_ref()
			.and_then(utils::get_data::<Weak<PanelItem<XdgBackend>>>)
			.as_deref()
			.and_then(Weak::upgrade)
			.map(|i| i.uid.clone());
		ToplevelInfo {
			parent,
			title: value.title.lock().clone(),
			app_id: value.app_id.lock().clone(),
			size,
			min_size: value.min_size.lock().clone(),
			max_size: value.max_size.lock().clone(),
			logical_rectangle,
		}
	}
}
impl Drop for ToplevelData {
	fn drop(&mut self) {
		// let Some(panel_item) = self.panel_item.get().and_then(Weak::upgrade) else {
		// return;
		// };
		// panel_item.drop_toplevel();
	}
}

impl Dispatch<XdgToplevel, WlWeak<WlSurface>, WaylandState> for WaylandState {
	fn request(
		_state: &mut WaylandState,
		_client: &Client,
		xdg_toplevel: &XdgToplevel,
		request: xdg_toplevel::Request,
		wl_surface_resource: &WlWeak<WlSurface>,
		_dhandle: &DisplayHandle,
		_data_init: &mut DataInit<'_, WaylandState>,
	) {
		let Ok(wl_surface) = wl_surface_resource.upgrade() else {
			error!("Couldn't get the wayland surface of the xdg toplevel");
			return;
		};
		let Some(toplevel_data) = utils::get_data::<ToplevelData>(&wl_surface) else {
			error!("Couldn't get the XdgToplevel");
			return;
		};
		match request {
			xdg_toplevel::Request::SetParent { parent } => {
				debug!(?xdg_toplevel, ?parent, "Set XDG Toplevel parent");
				let Some(parent_xdg_toplevel) = parent else {
					*toplevel_data.parent.lock() = None;
					return;
				};
				let Some(parent_toplevel_data) = parent_xdg_toplevel.data::<ToplevelData>() else {
					error!("Couldn't get XDG toplevel parent data");
					return;
				};
				let Ok(parent_wl_surface) = parent_toplevel_data.wl_surface.upgrade() else {
					error!("Couldn't get XDG toplevel parent wl surface");
					return;
				};
				*toplevel_data.parent.lock() = Some(parent_wl_surface.downgrade());
				let Some(parent_panel_item) = parent_toplevel_data
					.panel_item
					.get()
					.and_then(Weak::upgrade)
				else {
					return;
				};
				let Some(panel_item) = get_data::<PanelItem<XdgBackend>>(&wl_surface) else {
					error!("Couldn't get the panel item");
					return;
				};
				panel_item.toplevel_parent_changed(&parent_panel_item.uid);
			}
			xdg_toplevel::Request::SetTitle { title } => {
				debug!(?xdg_toplevel, ?title, "Set XDG Toplevel title");
				*toplevel_data.title.lock() = (!title.is_empty()).then_some(title.clone());
				let Some(panel_item) = get_data::<PanelItem<XdgBackend>>(&wl_surface) else {
					error!("Couldn't get the panel item");
					return;
				};
				panel_item.toplevel_title_changed(&title);
			}
			xdg_toplevel::Request::SetAppId { app_id } => {
				debug!(?xdg_toplevel, ?app_id, "Set XDG Toplevel app ID");
				*toplevel_data.app_id.lock() = (!app_id.is_empty()).then_some(app_id.clone());
				let Some(panel_item) = get_data::<PanelItem<XdgBackend>>(&wl_surface) else {
					error!("Couldn't get the panel item");
					return;
				};
				panel_item.toplevel_app_id_changed(&app_id);
			}
			xdg_toplevel::Request::Move { seat, serial } => {
				debug!(?xdg_toplevel, ?seat, serial, "XDG Toplevel move request");
				let Some(panel_item) = get_data::<PanelItem<XdgBackend>>(&wl_surface) else {
					error!("Couldn't get the panel item");
					return;
				};
				panel_item.toplevel_move_request();
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
				let (up, down, left, right) = match edges {
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
				let Some(panel_item) = get_data::<PanelItem<XdgBackend>>(&wl_surface) else {
					error!("Couldn't get the panel item");
					return;
				};
				panel_item.toplevel_resize_request(up, down, left, right)
			}
			xdg_toplevel::Request::SetMaxSize { width, height } => {
				debug!(?xdg_toplevel, width, height, "Set XDG Toplevel max size");
				*toplevel_data.max_size.lock() = (width > 1 || height > 1)
					.then_some(Vector2::from([width as u32, height as u32]));
			}
			xdg_toplevel::Request::SetMinSize { width, height } => {
				debug!(?xdg_toplevel, width, height, "Set XDG Toplevel min size");
				*toplevel_data.min_size.lock() = (width > 1 || height > 1)
					.then_some(Vector2::from([width as u32, height as u32]));
			}
			xdg_toplevel::Request::SetFullscreen { output: _ } => {
				let Some(panel_item) = get_data::<PanelItem<XdgBackend>>(&wl_surface) else {
					error!("Couldn't get the panel item");
					return;
				};
				panel_item.backend.toplevel_state.lock().fullscreen = true;
				panel_item.backend.configure(None);
				panel_item.toplevel_fullscreen_active(true);
			}
			xdg_toplevel::Request::UnsetFullscreen => {
				let Some(panel_item) = get_data::<PanelItem<XdgBackend>>(&wl_surface) else {
					error!("Couldn't get the panel item");
					return;
				};
				panel_item.backend.toplevel_state.lock().fullscreen = false;
				panel_item.backend.configure(None);
				panel_item.toplevel_fullscreen_active(false);
			}
			xdg_toplevel::Request::Destroy => {
				debug!(?xdg_toplevel, "Destroy XDG Toplevel");
				let Some(panel_item) = get_data::<PanelItem<XdgBackend>>(&wl_surface) else {
					error!("Couldn't get the panel item");
					return;
				};
				panel_item.backend.seat.drop_surface(&wl_surface);
				panel_item.drop_toplevel();
			}
			_ => {}
		}
	}
}

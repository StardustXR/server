use super::{backend::XdgBackend, positioner::PositionerData};
use crate::{
	nodes::items::panel::{Geometry, PanelItem, SurfaceID},
	wayland::{state::WaylandState, utils::get_data},
};
use parking_lot::Mutex;
use smithay::reexports::{
	wayland_protocols::xdg::shell::server::{
		xdg_popup::{self, XdgPopup},
		xdg_positioner::XdgPositioner,
	},
	wayland_server::{
		protocol::wl_surface::WlSurface, Client, DataInit, Dispatch, DisplayHandle, Resource,
		Weak as WlWeak,
	},
};
use std::sync::{Arc, Weak};
use tracing::{debug, error};
use wayland_backend::server::ClientId;

#[derive(Debug)]
pub struct PopupData {
	pub uid: String,
	grabbed: Mutex<bool>,
	parent: Mutex<WlWeak<WlSurface>>,
	panel_item: Weak<PanelItem<XdgBackend>>,
	positioner: Mutex<XdgPositioner>,
}
impl PopupData {
	pub fn new(
		uid: impl ToString,
		parent: WlSurface,
		panel_item: &Arc<PanelItem<XdgBackend>>,
		positioner: XdgPositioner,
	) -> Self {
		PopupData {
			uid: uid.to_string(),
			grabbed: Mutex::new(false),
			parent: Mutex::new(parent.downgrade()),
			panel_item: Arc::downgrade(panel_item),
			positioner: Mutex::new(positioner),
		}
	}
	pub fn geometry(&self) -> Option<Geometry> {
		let positioner = self.positioner.lock().clone();
		let positioner_data = positioner.data::<Mutex<PositionerData>>()?.lock();
		Some(positioner_data.clone().into())
	}
	pub fn parent(&self) -> WlSurface {
		self.parent.lock().upgrade().unwrap()
	}
}

impl Dispatch<XdgPopup, WlWeak<WlSurface>, WaylandState> for WaylandState {
	fn request(
		_state: &mut WaylandState,
		_client: &Client,
		xdg_popup: &XdgPopup,
		request: xdg_popup::Request,
		wl_surface_resource: &WlWeak<WlSurface>,
		_dhandle: &DisplayHandle,
		_data_init: &mut DataInit<'_, WaylandState>,
	) {
		let Ok(wl_surface) = wl_surface_resource.upgrade() else {
			error!("Couldn't get the wayland surface of the xdg popup");
			return;
		};
		let Some(popup_data) = get_data::<PopupData>(&wl_surface) else {
			error!("Couldn't get the XdgPopup");
			return;
		};
		let Some(panel_item) = popup_data.panel_item.upgrade() else {
			error!("Couldn't get the panel item");
			return;
		};
		match request {
			xdg_popup::Request::Grab { seat, serial } => {
				*popup_data.grabbed.lock() = true;
				debug!(?xdg_popup, ?seat, serial, "XDG popup grab");
				panel_item.grab_keyboard(Some(SurfaceID::Child(popup_data.uid.clone())));
			}
			xdg_popup::Request::Reposition { positioner, token } => {
				debug!(?xdg_popup, ?positioner, token, "XDG popup reposition");
				*popup_data.positioner.lock() = positioner;
				panel_item
					.backend
					.reposition_popup(&panel_item, &popup_data);
			}
			xdg_popup::Request::Destroy => {
				debug!(?xdg_popup, "Destroy XDG popup");
				if *popup_data.grabbed.lock() {
					panel_item.grab_keyboard(None);
				}
			}
			_ => unreachable!(),
		}
	}

	fn destroyed(
		_state: &mut WaylandState,
		_client: ClientId,
		_popup: &XdgPopup,
		data: &WlWeak<WlSurface>,
	) {
		let Ok(wl_surface) = data.upgrade() else {
			error!("Couldn't get the wayland surface of the xdg popup");
			return;
		};
		let Some(popup_data) = get_data::<PopupData>(&wl_surface) else {
			error!("Couldn't get the XdgPopup");
			return;
		};
		let Some(panel_item) = popup_data.panel_item.upgrade() else {
			error!("Couldn't get the panel item");
			return;
		};
		panel_item.backend.drop_popup(&panel_item, &popup_data.uid);
	}
}

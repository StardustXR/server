use self::{backend::XdgBackend, toplevel::ToplevelData};
use super::state::WaylandState;
use crate::wayland::{
	utils::insert_data,
	xdg_shell::{positioner::PositionerData, surface::XdgSurfaceData},
};
use parking_lot::Mutex;
use smithay::reexports::{
	wayland_protocols::xdg::shell::server::xdg_wm_base::{self, XdgWmBase},
	wayland_server::{Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource},
};
use tracing::debug;

mod backend;
mod popup;
mod positioner;
mod surface;
mod toplevel;

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
				let positioner = data_init.init(id, Mutex::new(PositionerData::default()));
				debug!(?positioner, "Create XDG positioner");
			}
			xdg_wm_base::Request::GetXdgSurface { id, surface } => {
				let xdg_surface = data_init.init(id, surface.downgrade());
				debug!(?xdg_surface, "Create XDG surface");
				insert_data(
					&surface,
					XdgSurfaceData {
						wl_surface: surface.downgrade(),
						xdg_surface,
						geometry: Mutex::new(None),
					},
				);
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

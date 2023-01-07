use super::state::WaylandState;
use smithay::{
	delegate_kde_decoration,
	reexports::{
		wayland_protocols::xdg::{
			decoration::zv1::server::{
				zxdg_decoration_manager_v1::{self, ZxdgDecorationManagerV1},
				zxdg_toplevel_decoration_v1::{self, Mode, ZxdgToplevelDecorationV1},
			},
			shell::server::xdg_toplevel::XdgToplevel,
		},
		wayland_protocols_misc::server_decoration::server::org_kde_kwin_server_decoration::{
			Mode as KdeMode, OrgKdeKwinServerDecoration,
		},
		wayland_server::{
			protocol::wl_surface::WlSurface, Client, DataInit, Dispatch, DisplayHandle,
			GlobalDispatch, New, Resource, WEnum, Weak,
		},
	},
	wayland::shell::{self, kde::decoration::KdeDecorationHandler},
};

impl GlobalDispatch<ZxdgDecorationManagerV1, (), WaylandState> for WaylandState {
	fn bind(
		_state: &mut WaylandState,
		_handle: &DisplayHandle,
		_client: &Client,
		resource: New<ZxdgDecorationManagerV1>,
		_global_data: &(),
		data_init: &mut DataInit<'_, WaylandState>,
	) {
		data_init.init(resource, ());
	}
}

impl Dispatch<ZxdgDecorationManagerV1, (), WaylandState> for WaylandState {
	fn request(
		_state: &mut WaylandState,
		_client: &Client,
		_resource: &ZxdgDecorationManagerV1,
		request: zxdg_decoration_manager_v1::Request,
		_data: &(),
		_dhandle: &DisplayHandle,
		data_init: &mut DataInit<'_, WaylandState>,
	) {
		match request {
			zxdg_decoration_manager_v1::Request::Destroy => (),
			zxdg_decoration_manager_v1::Request::GetToplevelDecoration { id, toplevel } => {
				data_init.init(id, toplevel.downgrade());
			}
			_ => unreachable!(),
		}
	}
}
impl Dispatch<ZxdgToplevelDecorationV1, Weak<XdgToplevel>, WaylandState> for WaylandState {
	fn request(
		_state: &mut WaylandState,
		_client: &Client,
		resource: &ZxdgToplevelDecorationV1,
		request: zxdg_toplevel_decoration_v1::Request,
		_data: &Weak<XdgToplevel>,
		_dhandle: &DisplayHandle,
		_data_init: &mut DataInit<'_, WaylandState>,
	) {
		match request {
			zxdg_toplevel_decoration_v1::Request::SetMode { mode: _ } => {
				resource.configure(Mode::ServerSide);
			}
			zxdg_toplevel_decoration_v1::Request::UnsetMode => {
				resource.configure(Mode::ServerSide);
			}
			zxdg_toplevel_decoration_v1::Request::Destroy => (),
			_ => unreachable!(),
		}
	}
}

impl KdeDecorationHandler for WaylandState {
	fn kde_decoration_state(&self) -> &shell::kde::decoration::KdeDecorationState {
		&self.kde_decoration_state
	}

	fn new_decoration(&mut self, _surface: &WlSurface, decoration: &OrgKdeKwinServerDecoration) {
		decoration.mode(KdeMode::Server);
	}

	fn request_mode(
		&mut self,
		_surface: &WlSurface,
		decoration: &OrgKdeKwinServerDecoration,
		_mode: WEnum<KdeMode>,
	) {
		decoration.mode(KdeMode::Server);
	}
}
delegate_kde_decoration!(WaylandState);

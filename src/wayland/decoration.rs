use super::{state::WaylandState, xdg_shell::XdgSurfaceData};
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
		wayland_server::{
			Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, Weak,
		},
	},
	wayland::shell::{self, kde::decoration::KdeDecorationHandler},
};

// impl XdgDecorationHandler for WaylandState {
// 	fn new_decoration(&mut self, toplevel: smithay::wayland::shell::xdg::ToplevelSurface) {
// 		toplevel.with_pending_state(|state| {
// 			state.decoration_mode = Some(Mode::ServerSide);
// 		});
// 		toplevel.send_configure();
// 	}

// 	fn request_mode(
// 		&mut self,
// 		_toplevel: smithay::wayland::shell::xdg::ToplevelSurface,
// 		_mode: smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode,
// 	) {
// 	}

// 	fn unset_mode(&mut self, _toplevel: smithay::wayland::shell::xdg::ToplevelSurface) {}
// }
// delegate_xdg_decoration!(WaylandState);

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
			zxdg_decoration_manager_v1::Request::Destroy => todo!(),
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
		data: &Weak<XdgToplevel>,
		_dhandle: &DisplayHandle,
		_data_init: &mut DataInit<'_, WaylandState>,
	) {
		match request {
			zxdg_toplevel_decoration_v1::Request::SetMode { mode: _ } => {
				resource.configure(Mode::ServerSide);
				data.upgrade()
					.unwrap()
					.data::<XdgSurfaceData>()
					.unwrap()
					.xdg_surface
					.upgrade()
					.unwrap()
					.configure(0);
			}
			zxdg_toplevel_decoration_v1::Request::UnsetMode => {
				resource.configure(Mode::ServerSide);
				data.upgrade()
					.unwrap()
					.data::<XdgSurfaceData>()
					.unwrap()
					.xdg_surface
					.upgrade()
					.unwrap()
					.configure(0);
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
}
delegate_kde_decoration!(WaylandState);

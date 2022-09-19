use super::state::WaylandState;
use smithay::{
	delegate_kde_decoration, delegate_xdg_decoration,
	reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode,
	wayland::shell::{
		self, kde::decoration::KdeDecorationHandler, xdg::decoration::XdgDecorationHandler,
	},
};

impl XdgDecorationHandler for WaylandState {
	fn new_decoration(&mut self, toplevel: smithay::wayland::shell::xdg::ToplevelSurface) {
		toplevel.with_pending_state(|state| {
			state.decoration_mode = Some(Mode::ServerSide);
		});
		toplevel.send_configure();
	}

	fn request_mode(
		&mut self,
		_toplevel: smithay::wayland::shell::xdg::ToplevelSurface,
		_mode: smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode,
	) {
	}

	fn unset_mode(&mut self, _toplevel: smithay::wayland::shell::xdg::ToplevelSurface) {}
}
delegate_xdg_decoration!(WaylandState);

impl KdeDecorationHandler for WaylandState {
	fn kde_decoration_state(&self) -> &shell::kde::decoration::KdeDecorationState {
		&self.kde_decoration_state
	}
}
delegate_kde_decoration!(WaylandState);

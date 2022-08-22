use smithay::{
	delegate_xdg_shell,
	reexports::wayland_protocols::xdg::{
		decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode,
		shell::server::xdg_toplevel::State,
	},
	wayland::shell::xdg::XdgShellHandler,
};

use super::WaylandState;

impl XdgShellHandler for WaylandState {
	fn xdg_shell_state(&mut self) -> &mut smithay::wayland::shell::xdg::XdgShellState {
		&mut self.xdg_shell_state
	}

	fn new_toplevel(
		&mut self,
		_dh: &smithay::reexports::wayland_server::DisplayHandle,
		surface: smithay::wayland::shell::xdg::ToplevelSurface,
	) {
		self.output
			.enter(&self.display_handle, surface.wl_surface());
		surface.with_pending_state(|state| {
			state.states.set(State::Fullscreen);
			state.decoration_mode = Some(Mode::ServerSide);
		});
		surface.send_configure();
	}

	fn new_popup(
		&mut self,
		_dh: &smithay::reexports::wayland_server::DisplayHandle,
		surface: smithay::wayland::shell::xdg::PopupSurface,
		_positioner: smithay::wayland::shell::xdg::PositionerState,
	) {
		self.output
			.enter(&self.display_handle, surface.wl_surface());
		let _ = surface.send_configure();
	}

	fn grab(
		&mut self,
		_dh: &smithay::reexports::wayland_server::DisplayHandle,
		_surface: smithay::wayland::shell::xdg::PopupSurface,
		_seat: smithay::reexports::wayland_server::protocol::wl_seat::WlSeat,
		_serial: smithay::wayland::Serial,
	) {
		todo!()
	}
}
delegate_xdg_shell!(WaylandState);

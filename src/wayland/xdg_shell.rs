use super::WaylandState;
use smithay::{
	delegate_xdg_shell,
	reexports::{
		wayland_protocols::xdg::{
			decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode,
			shell::server::xdg_toplevel::State,
		},
		wayland_server::protocol::wl_seat::WlSeat,
	},
	utils::Serial,
	wayland::shell::xdg::{
		PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
	},
};

impl XdgShellHandler for WaylandState {
	fn xdg_shell_state(&mut self) -> &mut XdgShellState {
		&mut self.xdg_shell_state
	}

	fn new_toplevel(&mut self, surface: ToplevelSurface) {
		self.output
			.enter(&self.display_handle, surface.wl_surface());
		surface.with_pending_state(|state| {
			state.states.set(State::Fullscreen);
			state.decoration_mode = Some(Mode::ServerSide);
		});
		surface.send_configure();
	}

	fn new_popup(&mut self, _surface: PopupSurface, _positioner: PositionerState) {}

	fn grab(&mut self, _surface: PopupSurface, _seat: WlSeat, _serial: Serial) {
		todo!()
	}
}
delegate_xdg_shell!(WaylandState);

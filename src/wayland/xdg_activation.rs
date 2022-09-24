use smithay::{
	delegate_xdg_activation,
	reexports::wayland_server::protocol::wl_surface::WlSurface,
	wayland::xdg_activation::{XdgActivationHandler, XdgActivationToken, XdgActivationTokenData},
};

use super::state::WaylandState;

impl XdgActivationHandler for WaylandState {
	fn activation_state(&mut self) -> &mut smithay::wayland::xdg_activation::XdgActivationState {
		&mut self.xdg_activation_state
	}

	fn request_activation(
		&mut self,
		token: XdgActivationToken,
		token_data: XdgActivationTokenData,
		_surface: WlSurface,
	) {
		dbg!(token);
		dbg!(token_data);
	}

	fn destroy_activation(
		&mut self,
		token: XdgActivationToken,
		token_data: XdgActivationTokenData,
		_surface: WlSurface,
	) {
		dbg!(token);
		dbg!(token_data);
	}
}
delegate_xdg_activation!(WaylandState);

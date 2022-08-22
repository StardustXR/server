use super::WaylandState;
use smithay::{
	backend::renderer::utils::{import_surface_tree, on_commit_buffer_handler},
	delegate_compositor,
	wayland::compositor::CompositorHandler,
};

impl CompositorHandler for WaylandState {
	fn compositor_state(&mut self) -> &mut smithay::wayland::compositor::CompositorState {
		&mut self.compositor_state
	}

	fn commit(
		&mut self,
		_dh: &smithay::reexports::wayland_server::DisplayHandle,
		surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
	) {
		on_commit_buffer_handler(surface);
		import_surface_tree(&mut self.renderer, surface, &self.log).unwrap();
	}
}

delegate_compositor!(WaylandState);

use super::{surface::CoreSurface, WaylandState};
use send_wrapper::SendWrapper;
use smithay::{
	backend::renderer::utils::{
		import_surface_tree, on_commit_buffer_handler, RendererSurfaceStateUserData,
	},
	delegate_compositor,
	wayland::compositor::{self, CompositorHandler},
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

		compositor::with_states(surface, |data| {
			if let Some(surface_states) = data.data_map.get::<RendererSurfaceStateUserData>() {
				if let Some(core_surface) = data.data_map.get::<CoreSurface>() {
					core_surface.wl_tex.replace(
						surface_states
							.borrow()
							.texture(&self.renderer)
							.cloned()
							.map(SendWrapper::new),
					);
				}
			}
		});
	}
}

delegate_compositor!(WaylandState);

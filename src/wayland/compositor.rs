use super::{state::WaylandState, surface::CoreSurface};
use smithay::{
	delegate_compositor,
	reexports::wayland_server::protocol::wl_surface::WlSurface,
	wayland::compositor::{self, CompositorHandler, CompositorState},
};

impl CompositorHandler for WaylandState {
	fn compositor_state(&mut self) -> &mut CompositorState {
		&mut self.compositor_state
	}

	fn commit(&mut self, surface: &WlSurface) {
		compositor::with_states(&surface, |data| {
			data.data_map.insert_if_missing_threadsafe(|| {
				CoreSurface::new(&self.display, self.display_handle.clone(), &surface)
			})
		});
	}
}

delegate_compositor!(WaylandState);

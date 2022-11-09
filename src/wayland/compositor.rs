use super::{state::WaylandState, surface::CoreSurface};
use smithay::{
	backend::renderer::utils::on_commit_buffer_handler,
	delegate_compositor,
	reexports::wayland_server::protocol::wl_surface::WlSurface,
	wayland::compositor::{self, CompositorHandler, CompositorState},
};

impl CompositorHandler for WaylandState {
	fn compositor_state(&mut self) -> &mut CompositorState {
		&mut self.compositor_state
	}

	fn commit(&mut self, surface: &WlSurface) {
		on_commit_buffer_handler(surface);
		compositor::with_states(surface, |data| {
			data.data_map.insert_if_missing_threadsafe(|| {
				CoreSurface::new(
					&self.weak_ref.upgrade().unwrap(),
					&self.display,
					self.display_handle.clone(),
					surface,
				)
			})
		});
	}
}

delegate_compositor!(WaylandState);

use super::{panel_item::PanelItem, state::WaylandState, surface::CoreSurface};
use smithay::{
	delegate_compositor,
	reexports::wayland_server::protocol::wl_surface::WlSurface,
	wayland::compositor::{self, CompositorHandler, CompositorState},
};
use std::sync::Arc;
use tracing::debug;

impl CompositorHandler for WaylandState {
	fn compositor_state(&mut self) -> &mut CompositorState {
		&mut self.compositor_state
	}

	fn commit(&mut self, surface: &WlSurface) {
		debug!(?surface, "Surface commit");
		CoreSurface::add_to(&self.display, self.display_handle.clone(), surface);
		if let Some(panel_item) = compositor::with_states(surface, |data| {
			data.data_map.get::<Arc<PanelItem>>().cloned()
		}) {
			panel_item.commit_toplevel();
		};
	}
}

delegate_compositor!(WaylandState);

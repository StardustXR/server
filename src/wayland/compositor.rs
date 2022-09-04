use super::state::WaylandState;
use crate::nodes::{core::Node, item::ItemType};
use smithay::{
	delegate_compositor,
	reexports::wayland_server::protocol::wl_surface::WlSurface,
	wayland::compositor::{self, CompositorHandler, CompositorState},
};
use std::sync::Arc;

impl CompositorHandler for WaylandState {
	fn compositor_state(&mut self) -> &mut CompositorState {
		&mut self.compositor_state
	}

	fn commit(&mut self, surface: &WlSurface) {
		compositor::with_states(surface, |data| {
			if let Some(panel_node) = data.data_map.get::<Arc<Node>>() {
				let item = panel_node.item.get().unwrap();
				if let ItemType::Panel(panel_item) = &item.specialization {
					panel_item.resize(&data.data_map);
				}
			}
		});
	}
}

delegate_compositor!(WaylandState);

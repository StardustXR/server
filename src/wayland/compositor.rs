use super::state::{ClientState, WaylandState};
use crate::wayland::surface::CoreSurface;
use portable_atomic::{AtomicU32, Ordering};
use smithay::{
	backend::renderer::utils::on_commit_buffer_handler,
	delegate_compositor,
	reexports::wayland_server::{protocol::wl_surface::WlSurface, Client},
	wayland::compositor::{self, CompositorClientState, CompositorHandler, CompositorState},
};
use std::sync::Arc;
use tracing::debug;

impl CompositorHandler for WaylandState {
	fn compositor_state(&mut self) -> &mut CompositorState {
		&mut self.compositor_state
	}

	fn commit(&mut self, surface: &WlSurface) {
		debug!(?surface, "Surface commit");

		on_commit_buffer_handler::<WaylandState>(surface);
		let mut count = 0;
		compositor::with_states(surface, |data| {
			let count_new = data
				.data_map
				.insert_if_missing_threadsafe(|| AtomicU32::new(0));
			if !count_new {
				if let Some(stored_count) = data.data_map.get::<AtomicU32>() {
					count = stored_count.fetch_add(1, Ordering::Relaxed);
				}
			}

			data.data_map.get::<Arc<CoreSurface>>().cloned()
		});
	}

	fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
		&client.get_data::<ClientState>().unwrap().compositor_state
	}

	// fn new_subsurface(&mut self, surface: &WlSurface, parent: &WlSurface) {
	// 	let Some(panel_item) = surface_panel_item(parent) else {
	// 		return;
	// 	};
	// 	let uid = surface.insert_data(Arc::downgrade(&panel_item));
	// }

	// fn destroyed(&mut self, surface: &WlSurface) {
	// 	let Some(panel_item) = surface_panel_item(surface) else {
	// 		return;
	// 	};
	// 	let Some((id, _)) = panel_item
	// 		.backend
	// 		.subsurfaces
	// 		.lock()
	// 		.iter()
	// 		.find(|(_, d)| *d == surface)
	// 	else {
	// 		return;
	// 	};
	// 	panel_item.backend.drop_subsurface(*id);

	// 	// self..lock().insert(id, (popup, positioner));

	// 	let child_data = self.child_data(id).unwrap();
	// 	panel_item.create_child(id, &child_data);
	// }
}

delegate_compositor!(WaylandState);

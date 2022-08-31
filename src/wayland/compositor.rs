use std::sync::Arc;

use crate::nodes::{core::Node, item::ItemType};

use super::{panel_item::PanelItem, surface::CoreSurface, WaylandState};
use send_wrapper::SendWrapper;
use smithay::{
	backend::renderer::utils::{
		import_surface_tree, on_commit_buffer_handler, RendererSurfaceStateUserData,
	},
	delegate_compositor,
	reexports::wayland_server::{protocol::wl_surface::WlSurface, DisplayHandle},
	wayland::compositor::{self, CompositorHandler, CompositorState},
};

impl CompositorHandler for WaylandState {
	fn compositor_state(&mut self) -> &mut CompositorState {
		&mut self.compositor_state
	}

	fn commit(&mut self, dh: &DisplayHandle, surface: &WlSurface) {
		on_commit_buffer_handler(surface);
		import_surface_tree(&mut self.renderer, surface, &self.log).unwrap();

		compositor::with_states(surface, |data| {
			let mapped = data
				.data_map
				.get::<RendererSurfaceStateUserData>()
				.map(|surface_states| surface_states.borrow().wl_buffer().is_some())
				.unwrap_or_default();

			if !mapped {
				return;
			}

			data.data_map.insert_if_missing_threadsafe(CoreSurface::new);
			data.data_map.insert_if_missing_threadsafe(|| {
				PanelItem::create(dh, &data.data_map, surface.clone())
			});

			let surface_states = data.data_map.get::<RendererSurfaceStateUserData>().unwrap();
			let core_surface = data.data_map.get::<CoreSurface>().unwrap();
			*core_surface.wl_tex.lock() = surface_states
				.borrow()
				.texture(&self.renderer)
				.cloned()
				.map(SendWrapper::new);

			if let ItemType::Panel(panel_item) = &data
				.data_map
				.get::<Arc<Node>>()
				.unwrap()
				.item
				.get()
				.unwrap()
				.specialization
			{
				panel_item.resize(&data.data_map);
			}
		});
	}
}

delegate_compositor!(WaylandState);

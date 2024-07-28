use super::{
	state::{ClientState, WaylandState},
	utils::WlSurfaceExt,
	xdg_shell::{surface_panel_item, ChildInfoExt},
};
use crate::{
	nodes::items::panel::{ChildInfo, Geometry, SurfaceId},
	wayland::surface::CoreSurface,
};
use parking_lot::Mutex;
use portable_atomic::{AtomicU32, Ordering};
use rand::Rng;
use smithay::{
	backend::renderer::utils::{on_commit_buffer_handler, RendererSurfaceStateUserData},
	delegate_compositor,
	reexports::wayland_server::{protocol::wl_surface::WlSurface, Client},
	wayland::compositor::{
		self, add_post_commit_hook, CompositorClientState, CompositorHandler, CompositorState,
	},
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

	fn new_subsurface(&mut self, surface: &WlSurface, parent: &WlSurface) {
		let id = rand::thread_rng().gen_range(0..u64::MAX);
		surface.insert_data(SurfaceId::Child(id));
		CoreSurface::add_to(surface);
		let Some(parent_surface_id) = parent.get_data::<SurfaceId>() else {
			return;
		};
		surface.insert_data(Mutex::new(ChildInfo {
			id,
			parent: parent_surface_id,
			geometry: Geometry {
				origin: [0; 2].into(),
				size: [256; 2].into(),
			},
			z_order: 1,
		}));

		let Some(panel_item) = surface_panel_item(parent) else {
			return;
		};
		let panel_item_weak = Arc::downgrade(&panel_item);
		add_post_commit_hook(surface, move |_: &mut WaylandState, _dh, surf| {
			if surface_panel_item(surf).is_some() {
				return;
			}
			surf.insert_data(panel_item_weak.clone());

			let Some(panel_item) = surface_panel_item(surf) else {
				return;
			};
			panel_item.backend.new_child(surf);
		});

		add_post_commit_hook(surface, move |_: &mut WaylandState, _dh, surf| {
			let Some(view) = surf
				.get_data_raw::<RendererSurfaceStateUserData, _, _>(|s| s.lock().ok()?.view())
				.flatten()
			else {
				return;
			};
			let mut changed = false;
			surf.with_child_info(|info| {
				if info.geometry.origin.x != view.offset.x
					&& info.geometry.origin.y != view.offset.y
				{
					changed = true;
				}
				if info.geometry.size.x != view.dst.w as u32
					&& info.geometry.size.y != view.dst.h as u32
				{
					changed = true;
				}
				info.geometry.size = [view.dst.w as u32, view.dst.h as u32].into();
			});

			let Some(panel_item) = surface_panel_item(surf) else {
				return;
			};
			if changed {
				panel_item.backend.reposition_child(surf);
			}
		});
	}

	fn destroyed(&mut self, surface: &WlSurface) {
		let Some(panel_item) = surface_panel_item(surface) else {
			return;
		};
		if surface.get_child_info().is_some() {
			panel_item.backend.drop_child(surface);
		}
	}
}

delegate_compositor!(WaylandState);

use super::{
	state::{ClientState, WaylandState},
	utils::{ChildInfoExt, WlSurfaceExt},
	xdg_shell::surface_panel_item,
};
use crate::{
	nodes::items::panel::{ChildInfo, Geometry, SurfaceId},
	wayland::surface::CoreSurface,
};
use parking_lot::Mutex;
use rand::Rng;
use smithay::{
	backend::renderer::utils::{on_commit_buffer_handler, RendererSurfaceStateUserData},
	delegate_compositor,
	desktop::PopupKind,
	reexports::wayland_server::{protocol::wl_surface::WlSurface, Client},
	wayland::compositor::{
		add_post_commit_hook, CompositorClientState, CompositorHandler, CompositorState,
	},
};
use std::sync::Arc;
use tracing::{debug, warn};

pub struct ConfiguredSurface;

impl CompositorHandler for WaylandState {
	fn compositor_state(&mut self) -> &mut CompositorState {
		&mut self.compositor_state
	}

	fn commit(&mut self, surface: &WlSurface) {
		debug!(?surface, "Surface commit");

		on_commit_buffer_handler::<WaylandState>(surface);

		if let Some(toplevel) = self
			.xdg_shell
			.toplevel_surfaces()
			.iter()
			.find(|s| s.wl_surface() == surface)
		{
			if !toplevel.is_initial_configure_sent() {
				debug!("Sending initial configure for toplevel surface");
				toplevel.send_configure();
				surface.insert_data(ConfiguredSurface);
			}
		}

		self.popup_manager.commit(surface);
		if let Some(PopupKind::Xdg(popup)) = self.popup_manager.find_popup(surface) {
			if surface.insert_data(ConfiguredSurface) {
				debug!("Configuring popup surface");
				let _ = popup.send_configure();
			}
		}
	}

	fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
		&client.get_data::<ClientState>().unwrap().compositor_state
	}

	fn new_subsurface(&mut self, surface: &WlSurface, parent: &WlSurface) {
		let id = rand::thread_rng().gen_range(0..u64::MAX);
		surface.insert_data(SurfaceId::Child(id));
		CoreSurface::add_to(surface);
		let Some(parent_surface_id) = parent.get_data::<SurfaceId>() else {
			warn!("Parent surface has no SurfaceId");
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
			receives_input: false,
		}));

		let Some(panel_item) = surface_panel_item(parent) else {
			warn!("Parent has no panel item");
			return;
		};
		let panel_item_weak = Arc::downgrade(&panel_item);
		add_post_commit_hook(surface, move |_: &mut WaylandState, _dh, surf| {
			if surface_panel_item(surf).is_some() {
				return;
			}
			debug!("Linking surface to panel item");
			surf.insert_data(panel_item_weak.clone());

			let Some(panel_item) = surface_panel_item(surf) else {
				warn!("Failed to link surface to panel item");
				return;
			};

			surf.with_child_info(|_info| {
				panel_item.backend.reposition_child(surf);
			});

			debug!("Adding new child to panel item");
			panel_item.backend.new_child(surf);
		});

		add_post_commit_hook(surface, move |_: &mut WaylandState, _dh, surf| {
			let Some(view) = surf
				.get_data_raw::<RendererSurfaceStateUserData, _, _>(|s| s.lock().ok()?.view())
				.flatten()
			else {
				debug!("No view data for surface");
				return;
			};
			let mut changed = false;
			surf.with_child_info(|info| {
				if info.geometry.origin.x != view.offset.x
					&& info.geometry.origin.y != view.offset.y
				{
					changed = true;
					debug!("Surface position changed");
				}
				if info.geometry.size.x != view.dst.w as u32
					&& info.geometry.size.y != view.dst.h as u32
				{
					changed = true;
					debug!("Surface size changed");
				}
				info.geometry.size = [view.dst.w as u32, view.dst.h as u32].into();
			});

			let Some(panel_item) = surface_panel_item(surf) else {
				return;
			};
			if changed {
				debug!("Repositioning child due to geometry change");
				panel_item.backend.reposition_child(surf);
			}
		});
	}

	fn destroyed(&mut self, surface: &WlSurface) {
		let Some(panel_item) = surface_panel_item(surface) else {
			return;
		};
		if surface.get_child_info().is_some() {
			debug!("Dropping destroyed child surface");
			panel_item.backend.drop_child(surface);
		}
	}
}

delegate_compositor!(WaylandState);

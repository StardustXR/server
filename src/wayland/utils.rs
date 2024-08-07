use mint::Vector2;
use parking_lot::Mutex;
use smithay::{
	backend::renderer::utils::RendererSurfaceStateUserData,
	reexports::wayland_server::protocol::wl_surface::WlSurface,
	wayland::{
		compositor,
		shell::xdg::{SurfaceCachedState, XdgToplevelSurfaceData},
	},
};

use crate::nodes::items::panel::{ChildInfo, Geometry, ToplevelInfo};

use super::xdg_shell::surface_panel_item;
pub trait WlSurfaceExt {
	fn insert_data<T: Send + Sync + 'static>(&self, data: T) -> bool;
	fn get_data<T: Send + Sync + Clone + 'static>(&self) -> Option<T>;
	fn get_data_raw<T: Send + Sync + 'static, O, F: FnOnce(&T) -> O>(&self, f: F) -> Option<O>;
	fn get_current_surface_state(&self) -> SurfaceCachedState;
	fn get_pending_surface_state(&self) -> SurfaceCachedState;
	fn get_size(&self) -> Option<Vector2<u32>>;
	fn get_geometry(&self) -> Option<Geometry>;
}
impl WlSurfaceExt for WlSurface {
	fn insert_data<T: Send + Sync + 'static>(&self, data: T) -> bool {
		compositor::with_states(self, |d| {
			d.data_map.insert_if_missing_threadsafe(move || data)
		})
	}
	fn get_data<T: Send + Sync + Clone + 'static>(&self) -> Option<T> {
		compositor::with_states(self, |d| d.data_map.get::<T>().cloned())
	}
	fn get_data_raw<T: Send + Sync + 'static, O, F: FnOnce(&T) -> O>(&self, f: F) -> Option<O> {
		compositor::with_states(self, |d| Some((f)(d.data_map.get::<T>()?)))
	}
	fn get_current_surface_state(&self) -> SurfaceCachedState {
		compositor::with_states(self, |states| {
			states
				.cached_state
				.get::<SurfaceCachedState>()
				.current()
				.clone()
		})
	}
	fn get_pending_surface_state(&self) -> SurfaceCachedState {
		compositor::with_states(self, |states| {
			states
				.cached_state
				.get::<SurfaceCachedState>()
				.pending()
				.clone()
		})
	}
	fn get_size(&self) -> Option<Vector2<u32>> {
		self.get_data_raw::<RendererSurfaceStateUserData, _, _>(|surface_states| {
			surface_states.lock().unwrap().surface_size()
		})
		.flatten()
		.map(|size| Vector2::from([size.w as u32, size.h as u32]))
	}
	fn get_geometry(&self) -> Option<Geometry> {
		self.get_current_surface_state().geometry.map(|r| r.into())
	}
}

pub trait ToplevelInfoExt {
	fn get_toplevel_info(&self) -> Option<ToplevelInfo>;
	fn with_toplevel_info<O, F: FnOnce(&mut ToplevelInfo) -> O>(&self, f: F) -> Option<O>;

	fn get_parent(&self) -> Option<u64>;
	fn get_app_id(&self) -> Option<String>;
	fn get_title(&self) -> Option<String>;
	fn min_size(&self) -> Option<Vector2<u32>>;
	fn max_size(&self) -> Option<Vector2<u32>>;
}
impl ToplevelInfoExt for WlSurface {
	fn get_toplevel_info(&self) -> Option<ToplevelInfo> {
		self.get_data_raw::<Mutex<ToplevelInfo>, _, _>(|c| c.lock().clone())
	}
	fn with_toplevel_info<O, F: FnOnce(&mut ToplevelInfo) -> O>(&self, f: F) -> Option<O> {
		self.get_data_raw::<Mutex<ToplevelInfo>, _, _>(|r| (f)(&mut r.lock()))
	}

	fn get_parent(&self) -> Option<u64> {
		self.get_data_raw::<XdgToplevelSurfaceData, _, _>(|d| d.lock().unwrap().parent.clone())
			.flatten()
			.and_then(|p| surface_panel_item(&p))
			.and_then(|p| p.node.upgrade())
			.map(|p| p.get_id())
	}
	fn get_app_id(&self) -> Option<String> {
		self.get_data_raw::<XdgToplevelSurfaceData, _, _>(|d| d.lock().ok()?.app_id.clone())
			.flatten()
	}
	fn get_title(&self) -> Option<String> {
		self.get_data_raw::<XdgToplevelSurfaceData, _, _>(|d| d.lock().ok()?.title.clone())
			.flatten()
	}
	fn min_size(&self) -> Option<Vector2<u32>> {
		let state = self.get_pending_surface_state();
		let size = state.min_size;
		if size.w == 0 && size.h == 0 {
			None
		} else {
			Some(Vector2::from([size.w as u32, size.h as u32]))
		}
	}
	fn max_size(&self) -> Option<Vector2<u32>> {
		let state = self.get_pending_surface_state();
		let size = state.max_size;
		if size.w == 0 && size.h == 0 {
			None
		} else {
			Some(Vector2::from([size.w as u32, size.h as u32]))
		}
	}
}
pub trait ChildInfoExt {
	fn get_child_info(&self) -> Option<ChildInfo>;
	fn with_child_info<O, F: FnOnce(&mut ChildInfo) -> O>(&self, f: F) -> Option<O>;
}
impl ChildInfoExt for WlSurface {
	fn get_child_info(&self) -> Option<ChildInfo> {
		self.get_data_raw::<Mutex<ChildInfo>, _, _>(|c| c.lock().clone())
	}
	fn with_child_info<O, F: FnOnce(&mut ChildInfo) -> O>(&self, f: F) -> Option<O> {
		self.get_data_raw::<Mutex<ChildInfo>, _, _>(|r| (f)(&mut r.lock()))
	}
}

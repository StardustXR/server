use smithay::{reexports::wayland_server::protocol::wl_surface::WlSurface, wayland::compositor};

pub trait WlSurfaceExt {
	fn insert_data<T: Send + Sync + 'static>(&self, data: T) -> bool;
	fn get_data<T: Send + Sync + Clone + 'static>(&self) -> Option<T>;
	fn get_data_raw<T: Send + Sync + 'static, O, F: FnOnce(&T) -> O>(&self, f: F) -> Option<O>;
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
}

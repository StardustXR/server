use smithay::{reexports::wayland_server::protocol::wl_surface::WlSurface, wayland::compositor};
use std::sync::Arc;

pub fn insert_data<T: Send + Sync + 'static>(wl_surface: &WlSurface, data: T) {
	insert_data_raw(wl_surface, Arc::new(data))
}
pub fn insert_data_raw<T: Send + Sync + 'static>(wl_surface: &WlSurface, data: Arc<T>) {
	compositor::with_states(wl_surface, |d| {
		d.data_map.insert_if_missing_threadsafe(move || data)
	});
}
pub fn get_data<T: Send + Sync + 'static>(wl_surface: &WlSurface) -> Option<Arc<T>> {
	compositor::with_states(wl_surface, |d| d.data_map.get::<Arc<T>>().cloned())
}

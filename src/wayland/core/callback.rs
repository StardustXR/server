use waynest::ObjectId;
pub use waynest_protocols::server::core::wayland::wl_callback::*;
use waynest_server::RequestDispatcher;

#[derive(Debug, RequestDispatcher, Clone)]
#[waynest(error = crate::wayland::WaylandError, connection = crate::wayland::Client)]
pub struct Callback(pub ObjectId);
/// https://wayland.app/protocols/wayland#wl_callback
impl WlCallback for Callback {
	type Connection = crate::wayland::Client;
}

pub use waynest::server::protocol::core::wayland::wl_callback::*;
use waynest::{
	server::{Dispatcher, Result},
	wire::ObjectId,
};

#[derive(Debug, Dispatcher, Clone)]
pub struct Callback(pub ObjectId);
/// https://wayland.app/protocols/wayland#wl_callback
impl WlCallback for Callback {}

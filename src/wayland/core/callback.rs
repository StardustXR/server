pub use waynest::server::protocol::core::wayland::wl_callback::*;
use waynest::{
	server::{Dispatcher, Result},
	wire::ObjectId,
};

#[derive(Debug, Dispatcher, Clone)]
pub struct Callback(pub ObjectId);
impl WlCallback for Callback {}

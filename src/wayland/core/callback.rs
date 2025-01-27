pub use waynest::server::protocol::core::wayland::wl_callback::*;
use waynest::server::{Dispatcher, Result};

#[derive(Debug, Dispatcher, Default)]
pub struct Callback;
impl WlCallback for Callback {}

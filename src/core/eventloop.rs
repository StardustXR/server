use libstardustxr::fusion::client::Client;
use libstardustxr::server;
use mio::net::UnixListener;
use slab::Slab;

pub struct EventLoop<'a> {
	pub socket_path: String,
	listener: UnixListener,
	clients: Slab<Client<'a>>,
}

impl<'a> EventLoop<'a> {
	pub fn new() -> Option<Self> {
		let socket_path = server::get_free_socket_path()?;
		Some(EventLoop {
			listener: UnixListener::bind(socket_path.clone()).ok()?,
			socket_path,
			clients: Slab::new(),
		})
	}
}

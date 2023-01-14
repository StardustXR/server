use super::client::Client;
use color_eyre::eyre::Result;
use once_cell::sync::OnceCell;
use stardust_xr::server;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use tokio::net::UnixListener;
use tokio::task::JoinHandle;

pub static FRAME: AtomicU64 = AtomicU64::new(0);

pub struct EventLoop {
	pub socket_path: PathBuf,
	join_handle: OnceCell<JoinHandle<()>>,
}

impl EventLoop {
	pub fn new() -> Result<Arc<Self>> {
		let socket_path = server::get_free_socket_path()
			.ok_or_else(|| std::io::Error::from(std::io::ErrorKind::Other))?;
		let socket = UnixListener::bind(socket_path.clone())?;

		let event_loop = Arc::new(EventLoop {
			socket_path,
			join_handle: OnceCell::new(),
		});

		let join_handle = tokio::task::Builder::new()
			.name("event loop")
			.spawn(async move {
				loop {
					let Ok((socket, _)) = socket.accept().await else { continue };
					Client::from_connection(socket);
				}
			})?;
		let _ = event_loop.join_handle.set(join_handle);

		Ok(event_loop)
	}
}

impl Drop for EventLoop {
	fn drop(&mut self) {
		if let Some(join_handle) = self.join_handle.take() {
			join_handle.abort();
		}
	}
}

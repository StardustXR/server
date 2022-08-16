use super::client::Client;
use anyhow::Result;
use libstardustxr::server;
use slab::Slab;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use tokio::net::UnixListener;
use tokio::sync::{Mutex, Notify, OnceCell};
use tokio::task::JoinHandle;

pub static FRAME: AtomicU64 = AtomicU64::new(0);

pub struct EventLoop {
	pub socket_path: String,
	stop_notifier: Arc<Notify>,
	pub clients: Mutex<Slab<OnceCell<Arc<Client>>>>,
}

impl EventLoop {
	pub fn new() -> Result<(Arc<Self>, JoinHandle<Result<()>>)> {
		let socket_path = server::get_free_socket_path()
			.ok_or_else(|| std::io::Error::from(std::io::ErrorKind::Other))?;
		let socket = UnixListener::bind(socket_path.clone())?;

		let event_loop = Arc::new(EventLoop {
			socket_path,
			stop_notifier: Default::default(),
			clients: Mutex::new(Slab::new()),
		});

		let event_loop_join_handle = tokio::spawn({
			let event_loop = event_loop.clone();
			async move { EventLoop::event_loop(socket, event_loop).await }
		});

		Ok((event_loop, event_loop_join_handle))
	}

	async fn event_loop(socket: UnixListener, event_loop: Arc<EventLoop>) -> Result<()> {
		let event_loop_async = async {
			loop {
				let (socket, _) = socket.accept().await?;
				let mut clients = event_loop.clients.lock().await;
				let idx = clients.insert(OnceCell::new());
				let _ = clients.get(idx).unwrap().set(Client::from_connection(
					idx,
					&event_loop,
					socket,
				));
			}
		};

		tokio::select! {
			_ = event_loop.stop_notifier.notified() => Ok(()),
			e = event_loop_async => e,
		}
	}
}

impl Drop for EventLoop {
	fn drop(&mut self) {
		self.stop_notifier.notify_one();
	}
}

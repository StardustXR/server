pub mod core;
pub mod util;
pub mod xdg;

use crate::core::{
	error::{Result, ServerError},
	task,
};
use cluFlock::ToFlock;
use core::display::Display;
use std::{
	fs::{self, OpenOptions},
	io::ErrorKind,
	os::unix::fs::OpenOptionsExt,
	path::PathBuf,
};
use tokio::{net::UnixStream, task::AbortHandle};
use tokio_stream::StreamExt;
use tracing::{debug_span, instrument};
use waynest::{
	server::{self, protocol::core::wayland::wl_display::WlDisplay},
	wire::ObjectId,
};

pub static WAYLAND_DISPLAY: OnceLock<String> = OnceLock::new();

impl From<waynest::server::Error> for ServerError {
	fn from(err: waynest::server::Error) -> Self {
		ServerError::WaylandError(err)
	}
}

pub fn get_free_wayland_socket_path() -> Option<PathBuf> {
	// Use XDG runtime directory for secure, user-specific sockets
	let base_dirs = directories::BaseDirs::new()?;
	let runtime_dir = base_dirs.runtime_dir()?;

	// Iterate through conventional display numbers (matches X11 behavior)
	for display in 0..=32 {
		let socket_path = runtime_dir.join(format!("wayland-{display}"));
		let socket_lock_path = runtime_dir.join(format!("wayland-{display}.lock"));

		// Open lock file without truncation to preserve existing locks
		let mut _lock = OpenOptions::new()
			.create(true)
			.truncate(false) // Prevent destroying other processes' locks
			.read(true)
			.write(true)
			.mode(0o660) // Match Wayland-compositor permissions
			.open(&socket_lock_path)
			.ok()?;

		// Atomic mutual exclusion: fail if another process holds the lock
		if _lock.try_exclusive_lock().is_err() {
			continue; // Lock held by active compositor
		}

		// Check for zombie sockets (file exists but nothing listening)
		if socket_path.exists() {
			match std::os::unix::net::UnixStream::connect(&socket_path) {
				Ok(_) => continue, // Active compositor found - skip
				Err(e) if e.kind() == ErrorKind::ConnectionRefused => {
					// Stale socket - safe to remove since we hold the lock
					let _ = fs::remove_file(&socket_path);
				}
				Err(_) => continue, // Transient error - conservative skip
			}
		}

		// Found viable candidate: lock held, socket cleared/available
		return Some(socket_path);
	}

	None // Exhausted all conventional display numbers
}

#[derive(Debug)]
struct WaylandClient {
	abort_handle: AbortHandle,
}
impl WaylandClient {
	pub fn from_stream(socket: UnixStream) -> Result<Self> {
		let mut client = server::Client::new(socket)?;
		client.insert(Display::default().into_object(ObjectId::DISPLAY));
		let abort_handle =
			task::new(|| "wayland client", Self::handle_client_messages(client))?.abort_handle();

		Ok(WaylandClient { abort_handle })
	}
	async fn handle_client_messages(mut client: server::Client) -> Result<()> {
		loop {
			match client.next_message().await {
				Ok(Some(mut msg)) => {
					if let Err(e) = client.handle_message(&mut msg).await {
						tracing::error!("Error handling message: {}", e);
						break;
					}
				}
				Err(e) => {
					tracing::error!("Error reading message: {}", e);
					break;
				}
				Ok(None) => {
					// Message stream ended
					break;
				}
			}
		}
		Ok(())
	}
}

impl Drop for WaylandClient {
	fn drop(&mut self) {
		self.abort_handle.abort();
	}
}

#[derive(Debug)]
pub struct Wayland {
	abort_handle: AbortHandle,
}
impl Wayland {
	pub fn new(socket_path: Option<PathBuf>) -> Result<Self> {
		let socket_path = if let Some(path) = socket_path {
			path
		} else {
			get_free_wayland_socket_path().ok_or(ServerError::WaylandError(
				waynest::server::Error::IoError(std::io::ErrorKind::AddrNotAvailable.into()),
			))?
		};

		let _ = WAYLAND_DISPLAY.set(socket_path.clone());

		let listener = server::Listener::new_with_path(&socket_path)
			.map_err(|e| ServerError::WaylandError(e))?;

		let abort_handle =
			task::new(|| "wayland loop", Self::handle_wayland_loop(listener))?.abort_handle();

		Ok(Self { abort_handle })
	}
	async fn handle_wayland_loop(mut listener: server::Listener) -> Result<()> {
		let mut clients = Vec::new();
		loop {
			if let Ok(Some(stream)) = listener.try_next().await {
				debug_span!("Accept wayland client").in_scope(|| {
					if let Ok(client) = WaylandClient::from_stream(stream) {
						clients.push(client);
					}
				});
			}
			clients.retain(|client| !client.abort_handle.is_finished());
		}

		#[allow(unreachable_code)]
		Ok(())
	}

	#[instrument(level = "debug", name = "Wayland frame", skip(self))]
	pub fn update(&mut self) {
		// TODO: Implement frame updates
	}

	pub fn frame_event(&self) {}
}

impl Drop for Wayland {
	fn drop(&mut self) {
		self.abort_handle.abort();
	}
}

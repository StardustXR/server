pub mod core;
pub mod dmabuf;
pub mod util;
pub mod xdg;

use crate::core::{
	error::{Result, ServerError},
	task,
};
use crate::wayland::core::seat::SeatMessage;
use cluFlock::ToFlock;
use core::{
	buffer::{BUFFER_REGISTRY, Buffer},
	callback::Callback,
	display::Display,
	surface::WL_SURFACE_REGISTRY,
};
use dmabuf::buffer_params::BufferParams;
use mint::Vector2;
use std::{
	fs::{self, OpenOptions},
	io::{self, ErrorKind},
	os::unix::fs::OpenOptionsExt,
	path::PathBuf,
	sync::{Arc, OnceLock},
};
use tokio::{net::UnixStream, sync::mpsc, task::AbortHandle};
use tokio_stream::StreamExt;
use tracing::{debug_span, instrument};
use waynest::{
	server::{
		self,
		protocol::{
			core::wayland::{wl_buffer::WlBuffer, wl_callback::WlCallback, wl_display::WlDisplay},
			stable::{
				linux_dmabuf_v1::zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1,
				xdg_shell::xdg_toplevel::XdgToplevel,
			},
		},
	},
	wire::{DecodeError, ObjectId},
};
use xdg::toplevel::Toplevel;

pub static WAYLAND_DISPLAY: OnceLock<PathBuf> = OnceLock::new();

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

pub enum Message {
	Frame(Arc<Callback>),
	ReleaseBuffer(Arc<Buffer>),
	DmabufImportSuccess(Arc<BufferParams>, Arc<Buffer>),
	DmabufImportFailure(Arc<BufferParams>),
	CloseToplevel(Arc<Toplevel>),
	ResizeToplevel {
		toplevel: Arc<Toplevel>,
		size: Option<Vector2<u32>>,
	},
	SetToplevelVisualActive {
		toplevel: Arc<Toplevel>,
		active: bool,
	},
	Seat(SeatMessage),
}

pub type MessageSink = mpsc::UnboundedSender<Message>;

#[derive(Debug)]
struct WaylandClient {
	abort_handle: AbortHandle,
}
impl WaylandClient {
	pub fn from_stream(socket: UnixStream) -> Result<Self> {
		let pid = socket.peer_cred().ok().and_then(|c| c.pid());
		let mut client = server::Client::new(socket)?;
		let (message_sink, message_source) = mpsc::unbounded_channel();

		client.insert(ObjectId::DISPLAY, Display::new(message_sink, pid));
		let abort_handle = task::new(
			|| "wayland client",
			Self::handle_client_messages(client, message_source),
		)?
		.abort_handle();

		Ok(WaylandClient { abort_handle })
	}
	async fn handle_client_messages(
		mut client: server::Client,
		mut render_message_rx: mpsc::UnboundedReceiver<Message>,
	) -> Result<()> {
		loop {
			tokio::select! {
				// send all queued up messages
				msg = render_message_rx.recv() => {
					if let Some(msg) = msg {
						Self::handle_render_message(&mut client, msg).await?;
					}
				}
				// handle the next message
				msg = client.next_message() => {
					match msg {
						Ok(Some(mut msg)) => {
							if let Err(e) = client.handle_message(&mut msg).await {
								tracing::error!("Wayland: Error handling message: {:?}", e);
								break;
							}
						}
						Err(e) => {
							// wayland clients really aren't nice when disconnecting properly, are they? :p
							if let server::Error::Decode(DecodeError::IoError(e)) = &e {
								if e.kind() == io::ErrorKind::ConnectionReset {
									if let Some(pid) = client.get::<Display>(ObjectId::DISPLAY).and_then(|d| d.pid) {
										tracing::info!("Wayland: Client with pid: {pid} disconnected from server");
									} else {
										tracing::info!("Wayland: Unknown client disconnected from server");
									}
									break;
								}
							}
							tracing::error!("Wayland: Error reading message: {:?}", e);
							break;
						}
						Ok(None) => {
							if let Some(pid) = client.get::<Display>(ObjectId::DISPLAY).and_then(|d| d.pid) {
								tracing::info!("Wayland: Client with pid: {pid} disconnected from server");
							} else {
								tracing::info!("Wayland: Unknown client disconnected from server");
							}
							// Message stream ended
							break;
						}
					}
				}
			}
		}
		Ok(())
	}

	async fn handle_render_message(
		client: &mut server::Client,
		message: Message,
	) -> Result<(), waynest::server::Error> {
		match message {
			Message::Frame(callback) => {
				let serial = client.next_event_serial();
				client
					.get::<Display>(ObjectId::DISPLAY)
					.unwrap()
					.delete_id(client, ObjectId::DISPLAY, callback.0.as_raw())
					.await?;
				client.remove(callback.0);
				callback.done(client, callback.0, serial).await
			}
			Message::DmabufImportSuccess(params, buffer) => {
				params.created(client, params.id, buffer.id).await
			}
			Message::DmabufImportFailure(params) => {
				client.remove(params.id);
				params.failed(client, params.id).await
			}
			Message::ReleaseBuffer(buffer) => buffer.release(client, buffer.id).await,
			Message::CloseToplevel(toplevel) => toplevel.close(client, toplevel.object_id).await,
			Message::ResizeToplevel { toplevel, size } => {
				toplevel.set_size(size);
				toplevel.reconfigure(client).await
			}
			Message::SetToplevelVisualActive { toplevel, active } => {
				toplevel.set_activated(active);
				toplevel.reconfigure(client).await
			}
			Message::Seat(seat_message) => {
				if let Some(seat) = client.get::<Display>(ObjectId::DISPLAY).unwrap().seat.get() {
					seat.handle_message(client, seat_message).await?;
				}
				Ok(())
			}
		}
	}
}
impl Drop for WaylandClient {
	fn drop(&mut self) {
		self.abort_handle.abort();
	}
}

#[derive(Debug)]
pub struct GraphicsInfo {
	pub egl_instance: khronos_egl::Instance<khronos_egl::Static>,
	pub display: khronos_egl::Display,
	pub context: khronos_egl::Context,
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

		let listener =
			server::Listener::new_with_path(&socket_path).map_err(ServerError::WaylandError)?;

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
	pub fn update_graphics(&mut self) {
		for surface in WL_SURFACE_REGISTRY.get_valid_contents() {
			surface.update_graphics();
		}
	}

	pub fn early_frame(&self, graphics_info: &GraphicsInfo) {
		for buffer in BUFFER_REGISTRY.get_valid_contents() {
			buffer.init_tex(graphics_info);
		}
		for surface in WL_SURFACE_REGISTRY.get_valid_contents() {
			surface.frame_event();
		}
	}
}
impl Drop for Wayland {
	fn drop(&mut self) {
		self.abort_handle.abort();
	}
}

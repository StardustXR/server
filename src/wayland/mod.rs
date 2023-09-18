mod compositor;
mod data_device;
mod decoration;
mod seat;
mod state;
mod surface;
// mod xdg_activation;
mod xdg_shell;
#[cfg(feature = "xwayland")]
pub mod xwayland;

use self::{state::WaylandState, surface::CORE_SURFACES};
use crate::core::buffers::BufferManager;
use crate::wayland::seat::SeatData;
use crate::{core::task, wayland::state::ClientState};
use color_eyre::eyre::Result;
use global_counter::primitive::exact::CounterU32;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use sk::StereoKitDraw;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::renderer::ImportDma;
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::reexports::wayland_server::{Display, ListeningSocket};
use std::ffi::OsStr;
use std::os::fd::OwnedFd;
use std::os::unix::prelude::AsRawFd;
use std::{
	os::unix::{net::UnixListener, prelude::FromRawFd},
	sync::Arc,
};
use stereokit as sk;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::{
	io::unix::AsyncFd, net::UnixListener as AsyncUnixListener, sync::mpsc, task::JoinHandle,
};
use tracing::{debug_span, info, instrument};

pub static WAYLAND_DISPLAY: OnceCell<String> = OnceCell::new();
pub static SERIAL_COUNTER: CounterU32 = CounterU32::new(0);

pub struct DisplayWrapper(Mutex<Display<WaylandState>>, DisplayHandle);
impl DisplayWrapper {
	pub fn handle(&self) -> DisplayHandle {
		self.1.clone()
	}
	pub fn dispatch_clients(&self, state: &mut WaylandState) -> Result<usize, std::io::Error> {
		self.0.lock().dispatch_clients(state)
	}
	pub fn flush_clients(&self, client: Option<ClientId>) {
		if let Some(mut lock) = self.0.try_lock() {
			let _ = lock.backend().flush(client);
		}
	}
	pub fn poll_fd(&self) -> Result<OwnedFd, std::io::Error> {
		self.0.lock().backend().poll_fd().try_clone_to_owned()
	}
}

pub struct Wayland {
	display: Arc<DisplayWrapper>,
	pub socket_name: Option<String>,
	join_handle: JoinHandle<Result<()>>,
	dmabuf_rx: UnboundedReceiver<Dmabuf>,
	wayland_state: Arc<Mutex<WaylandState>>,
	#[cfg(feature = "xwayland")]
	pub xwayland_state: xwayland::XWaylandState,
}
impl Wayland {
	pub fn new(buffer_manager: &BufferManager) -> Result<Self> {
		let display: Display<WaylandState> = Display::new()?;
		let display_handle = display.handle();

		let (dmabuf_tx, dmabuf_rx) = mpsc::unbounded_channel();
		let display = Arc::new(DisplayWrapper(Mutex::new(display), display_handle.clone()));
		#[cfg(feature = "xwayland")]
		let xwayland_state = xwayland::XWaylandState::create(&display_handle)?;
		let wayland_state = WaylandState::new(display_handle, &buffer_manager.renderer, dmabuf_tx);

		let socket = ListeningSocket::bind_auto("wayland", 0..33)?;
		let socket_name = socket
			.socket_name()
			.and_then(OsStr::to_str)
			.map(ToString::to_string);
		if let Some(socket_name) = &socket_name {
			let _ = WAYLAND_DISPLAY.set(socket_name.clone());
		}
		info!(socket_name, "Wayland active");

		let join_handle = Wayland::start_loop(display.clone(), socket, wayland_state.clone())?;

		Ok(Wayland {
			display,
			socket_name,
			join_handle,
			dmabuf_rx,
			wayland_state,
			#[cfg(feature = "xwayland")]
			xwayland_state,
		})
	}

	fn start_loop(
		display: Arc<DisplayWrapper>,
		socket: ListeningSocket,
		state: Arc<Mutex<WaylandState>>,
	) -> Result<JoinHandle<Result<()>>> {
		let listen_async =
			AsyncUnixListener::from_std(unsafe { UnixListener::from_raw_fd(socket.as_raw_fd()) })?;

		let dispatch_poll_fd = display.poll_fd()?;
		let dispatch_poll_listener = AsyncFd::new(dispatch_poll_fd)?;

		let dh1 = display.handle();
		let mut dh2 = dh1.clone();

		Ok(task::new(|| "wayland loop", async move {
			let _socket = socket; // Keep the socket alive
			loop {
				tokio::select! {
					acc = listen_async.accept() => { // New client connected
						let (stream, _) = acc?;
						let client_state = Arc::new(ClientState {
							id: OnceCell::new(),
							compositor_state: Default::default(),
							display: Arc::downgrade(&display),
							seat: SeatData::new(&dh1)
						});
						let client = dh2.insert_client(stream.into_std()?, client_state.clone())?;
						let _ = client_state.seat.client.set(client.id());
					}
					e = dispatch_poll_listener.readable() => { // Dispatch
						let mut guard = e?;
						debug_span!("Dispatch wayland event").in_scope(|| -> Result<(), color_eyre::Report> {
							display.dispatch_clients(&mut *state.lock())?;
							display.flush_clients(None);
							Ok(())
						})?;
						guard.clear_ready();
					}
				}
			}
		})?)
	}

	#[instrument(
		level = "debug",
		name = "Wayland frame",
		skip(self, sk, buffer_manager)
	)]
	pub fn update(&mut self, sk: &impl StereoKitDraw, buffer_manager: &mut BufferManager) {
		while let Ok(dmabuf) = self.dmabuf_rx.try_recv() {
			let _ = buffer_manager.renderer.import_dmabuf(&dmabuf, None);
		}
		for core_surface in CORE_SURFACES.get_valid_contents() {
			core_surface.process(sk, &mut buffer_manager.renderer);
		}

		self.display.flush_clients(None);
	}

	pub fn frame_event(&self, sk: &impl StereoKitDraw) {
		let output = self.wayland_state.lock().output.clone();

		for core_surface in CORE_SURFACES.get_valid_contents() {
			core_surface.frame(sk, output.clone());
		}
	}
}
impl Drop for Wayland {
	fn drop(&mut self) {
		self.join_handle.abort();
	}
}

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
use crate::wayland::seat::SeatData;
use crate::{core::task, wayland::state::ClientState};
use color_eyre::eyre::{ensure, Result};
use global_counter::primitive::exact::CounterU32;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use sk::StereoKitDraw;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::egl::EGLContext;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::ImportDma;
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::reexports::wayland_server::{backend::GlobalId, Display, ListeningSocket};
use std::os::fd::OwnedFd;
use std::os::unix::prelude::AsRawFd;
use std::{
	ffi::c_void,
	os::unix::{net::UnixListener, prelude::FromRawFd},
	sync::Arc,
};
use stereokit as sk;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::{
	io::unix::AsyncFd, net::UnixListener as AsyncUnixListener, sync::mpsc, task::JoinHandle,
};
use tracing::{debug, debug_span, info, instrument};

pub static WAYLAND_DISPLAY: OnceCell<String> = OnceCell::new();
pub static SERIAL_COUNTER: CounterU32 = CounterU32::new(0);

struct EGLRawHandles {
	display: *const c_void,
	config: *const c_void,
	context: *const c_void,
}
fn get_sk_egl() -> Result<EGLRawHandles> {
	ensure!(
		unsafe { sk::sys::backend_graphics_get() }
			== sk::sys::backend_graphics__backend_graphics_opengles_egl,
		"StereoKit is not running using EGL!"
	);

	Ok(unsafe {
		EGLRawHandles {
			display: sk::sys::backend_opengl_egl_get_display() as *const c_void,
			config: sk::sys::backend_opengl_egl_get_config() as *const c_void,
			context: sk::sys::backend_opengl_egl_get_context() as *const c_void,
		}
	})
}

static GLOBAL_DESTROY_QUEUE: OnceCell<mpsc::Sender<GlobalId>> = OnceCell::new();

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
	pub socket_name: String,
	join_handle: JoinHandle<Result<()>>,
	renderer: GlesRenderer,
	dmabuf_rx: UnboundedReceiver<Dmabuf>,
	wayland_state: Arc<Mutex<WaylandState>>,
	#[cfg(feature = "xwayland")]
	pub xwayland_state: xwayland::XWaylandState,
}
impl Wayland {
	pub fn new() -> Result<Self> {
		let egl_raw_handles = get_sk_egl()?;
		let renderer = unsafe {
			GlesRenderer::new(EGLContext::from_raw(
				egl_raw_handles.display,
				egl_raw_handles.config,
				egl_raw_handles.context,
			)?)?
		};

		let display: Display<WaylandState> = Display::new()?;
		let display_handle = display.handle();

		let (dmabuf_tx, dmabuf_rx) = mpsc::unbounded_channel();
		let display = Arc::new(DisplayWrapper(Mutex::new(display), display_handle.clone()));
		#[cfg(feature = "xwayland")]
		let xwayland_state = xwayland::XWaylandState::create(&display_handle)?;
		let wayland_state = WaylandState::new(display_handle, &renderer, dmabuf_tx);

		let (global_destroy_queue_in, global_destroy_queue) = mpsc::channel(8);
		GLOBAL_DESTROY_QUEUE.set(global_destroy_queue_in).unwrap();

		let socket = ListeningSocket::bind_auto("wayland", 0..33)?;
		let socket_name = socket.socket_name().unwrap().to_str().unwrap().to_string();
		WAYLAND_DISPLAY
			.set(socket_name.clone())
			.expect("seriously message nova this time they screwed up big time");
		info!(socket_name, "Wayland active");

		let join_handle = Wayland::start_loop(
			display.clone(),
			socket,
			wayland_state.clone(),
			global_destroy_queue,
		)?;

		Ok(Wayland {
			display,
			socket_name,
			join_handle,
			renderer,
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
		mut global_destroy_queue: mpsc::Receiver<GlobalId>,
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
					e = global_destroy_queue.recv() => { // New global to destroy
						debug!(?e, "destroy global");
						dh1.remove_global::<WaylandState>(e.unwrap());
					}
					acc = listen_async.accept() => { // New client connected
						let (stream, _) = acc?;
						let client_state = Arc::new(ClientState {
							id: OnceCell::new(),
							compositor_state: Default::default(),
							display: Arc::downgrade(&display),
							seat: SeatData::new(&dh1)
						});
						let client = dh2.insert_client(stream.into_std()?, client_state.clone())?;
						client_state.seat.client.set(client.id()).unwrap();
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

	#[instrument(level = "debug", name = "Wayland frame", skip(self, sk))]
	pub fn update(&mut self, sk: &impl StereoKitDraw) {
		while let Ok(dmabuf) = self.dmabuf_rx.try_recv() {
			let _ = self.renderer.import_dmabuf(&dmabuf, None);
		}
		for core_surface in CORE_SURFACES.get_valid_contents() {
			core_surface.process(sk, &mut self.renderer);
		}

		self.display.flush_clients(None);
	}

	pub fn frame_event(&self, sk: &impl StereoKitDraw) {
		let output = self.wayland_state.lock().output.clone();

		for core_surface in CORE_SURFACES.get_valid_contents() {
			core_surface.frame(sk, output.clone());
		}
	}

	pub fn make_context_current(&self) {
		unsafe {
			self.renderer.egl_context().make_current().unwrap();
		}
	}
}
impl Drop for Wayland {
	fn drop(&mut self) {
		self.join_handle.abort();
	}
}

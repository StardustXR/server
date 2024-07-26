mod compositor;
mod data_device;
mod decoration;
mod seat;
mod state;
mod surface;
// mod xdg_activation;
mod drm;
mod utils;
mod xdg_shell;

use self::{state::WaylandState, surface::CORE_SURFACES};
use crate::{core::task, wayland::state::ClientState};
use color_eyre::eyre::{ensure, Result};
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::egl::EGLContext;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::{ImportDma, Renderer};
use smithay::output::Output;
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::reexports::wayland_server::{Display, ListeningSocket};
use smithay::wayland::dmabuf;
use std::ffi::OsStr;
use std::os::fd::{IntoRawFd, OwnedFd};
use std::os::unix::prelude::AsRawFd;
use std::{
	ffi::c_void,
	os::unix::{net::UnixListener, prelude::FromRawFd},
	sync::Arc,
};
use stereokit_rust::system::{Backend, BackendGraphics};
use tokio::io::unix::AsyncFdReadyGuard;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::{
	io::unix::AsyncFd, net::UnixListener as AsyncUnixListener, sync::mpsc, task::JoinHandle,
};
use tracing::{debug_span, info, instrument};

pub static WAYLAND_DISPLAY: OnceCell<String> = OnceCell::new();

struct EGLRawHandles {
	display: *const c_void,
	config: *const c_void,
	context: *const c_void,
}
fn get_sk_egl() -> Result<EGLRawHandles> {
	ensure!(
		Backend::graphics() == BackendGraphics::OpenGLESEGL,
		"StereoKit is not running using EGL!"
	);

	Ok(unsafe {
		EGLRawHandles {
			display: stereokit_rust::system::backend_opengl_egl_get_display() as *const c_void,
			config: stereokit_rust::system::backend_opengl_egl_get_config() as *const c_void,
			context: stereokit_rust::system::backend_opengl_egl_get_context() as *const c_void,
		}
	})
}

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

struct UnownedFd(Option<AsyncFd<OwnedFd>>);
impl UnownedFd {
	async fn readable(&self) -> std::io::Result<AsyncFdReadyGuard<'_, OwnedFd>> {
		self.0.as_ref().unwrap().readable().await
	}
}
impl Drop for UnownedFd {
	fn drop(&mut self) {
		self.0.take().unwrap().into_inner().into_raw_fd();
	}
}

pub struct Wayland {
	display: Arc<DisplayWrapper>,
	pub socket_name: Option<String>,
	join_handle: JoinHandle<Result<()>>,
	renderer: GlesRenderer,
	output: Output,
	dmabuf_rx: UnboundedReceiver<(Dmabuf, Option<dmabuf::ImportNotifier>)>,
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

		let wayland_state = WaylandState::new(display_handle.clone(), &renderer, dmabuf_tx);
		let output = wayland_state.lock().output.clone();

		let socket = ListeningSocket::bind_auto("wayland", 0..33)?;
		let socket_name = socket
			.socket_name()
			.and_then(OsStr::to_str)
			.map(ToString::to_string);
		if let Some(socket_name) = &socket_name {
			let _ = WAYLAND_DISPLAY.set(socket_name.clone());
		}
		info!(socket_name, "Wayland active");

		let join_handle = Wayland::start_loop(display.clone(), socket, wayland_state)?;

		Ok(Wayland {
			display,
			socket_name,
			join_handle,
			renderer,
			output,
			dmabuf_rx,
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
		let dispatch_poll_listener = UnownedFd(Some(AsyncFd::new(dispatch_poll_fd)?));

		let dh1 = display.handle();
		let mut dh2 = dh1.clone();

		task::new(|| "wayland loop", async move {
			let _socket = socket; // Keep the socket alive
			loop {
				tokio::select! {
					acc = listen_async.accept() => { // New client connected
						let (stream, _) = acc?;
						let client_state = Arc::new(ClientState {
							pid: stream.peer_cred().ok().and_then(|c| c.pid()),
							id: OnceCell::new(),
							compositor_state: Default::default(),
							seat: state.lock().seat.clone(),
						});
						let _client = dh2.insert_client(stream.into_std()?, client_state.clone())?;
					}
					e = dispatch_poll_listener.readable() => { // Dispatch
						let mut guard = e?;
						debug_span!("Dispatch wayland event").in_scope(|| -> Result<(), color_eyre::Report> {
							display.dispatch_clients(&mut state.lock())?;
							display.flush_clients(None);
							Ok(())
						})?;
						guard.clear_ready();
					}
				}
			}
		})
	}

	#[instrument(level = "debug", name = "Wayland frame", skip(self))]
	pub fn update(&mut self) {
		while let Ok((dmabuf, notifier)) = self.dmabuf_rx.try_recv() {
			if self.renderer.import_dmabuf(&dmabuf, None).is_err() {
				if let Some(notifier) = notifier {
					notifier.failed();
				}
			}
		}
		for core_surface in CORE_SURFACES.get_valid_contents() {
			core_surface.process(&mut self.renderer);
		}
		let _ = self.renderer.cleanup_texture_cache();

		self.display.flush_clients(None);
	}

	pub fn frame_event(&self) {
		for core_surface in CORE_SURFACES.get_valid_contents() {
			core_surface.frame(self.output.clone());
		}
	}

	pub fn make_context_current(&self) {
		unsafe {
			let _ = self.renderer.egl_context().make_current();
		}
	}
}
impl Drop for Wayland {
	fn drop(&mut self) {
		self.join_handle.abort();
	}
}

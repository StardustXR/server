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
use color_eyre::eyre::{Result, ensure};
use parking_lot::Mutex;
use smithay::{
	backend::{
		allocator::dmabuf::Dmabuf,
		egl::EGLContext,
		renderer::{ImportDma, Renderer, gles::GlesRenderer},
	},
	output::Output,
	reexports::wayland_server::{Display, DisplayHandle, ListeningSocket},
	wayland::dmabuf,
};
use std::{
	ffi::{OsStr, c_void},
	os::fd::AsFd,
	sync::{Arc, OnceLock},
};
use stereokit_rust::system::{Backend, BackendGraphics};
use tokio::{
	io::unix::AsyncFd,
	sync::{
		Notify,
		mpsc::{self, UnboundedReceiver},
	},
	task::AbortHandle,
};
use tracing::{debug_span, info, instrument};

pub static WAYLAND_DISPLAY: OnceLock<String> = OnceLock::new();

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

pub struct Wayland {
	flush_notify: Arc<Notify>,
	client_listener: AbortHandle,
	client_dispatcher: AbortHandle,
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

		let flush_notify = Arc::new(Notify::new());
		let client_listener = task::new(
			|| "Wayland client listener loop",
			Wayland::client_listener_loop(display_handle, socket, wayland_state.clone()),
		)?
		.abort_handle();
		let client_dispatcher = task::new(
			|| "Wayland dispatch client loop",
			Wayland::dispatch_client_loop(display, flush_notify.clone(), wayland_state),
		)?
		.abort_handle();

		Ok(Wayland {
			flush_notify,
			client_listener,
			client_dispatcher,
			renderer,
			output,
			dmabuf_rx,
		})
	}

	async fn client_listener_loop(
		mut display_handle: DisplayHandle,
		socket: ListeningSocket,
		state: Arc<Mutex<WaylandState>>,
	) -> Result<()> {
		let async_fd = AsyncFd::new(socket.as_fd())?;
		loop {
			let mut guard = async_fd.readable().await?;
			let Ok(Some(stream)) = socket.accept() else {
				guard.clear_ready();
				continue;
			};

			let stream = tokio::net::UnixStream::from_std(stream)?;
			let pid = stream.peer_cred().ok().and_then(|c| c.pid());

			// New client connected
			let client_state = Arc::new(ClientState {
				pid,
				id: OnceLock::new(),
				compositor_state: Default::default(),
				seat: state.lock().seat.clone(),
			});
			let _client = display_handle.insert_client(stream.into_std()?, client_state.clone())?;
		}
	}

	async fn dispatch_client_loop(
		mut display: Display<WaylandState>,
		flush_notify: Arc<Notify>,
		state: Arc<Mutex<WaylandState>>,
	) -> std::io::Result<()> {
		loop {
			let poll_fd = display.backend().poll_fd();
			let async_fd = AsyncFd::new(poll_fd)?;
			tokio::select! {
				biased;
				_ = async_fd.readable() => {
					drop(async_fd);
					let _span = debug_span!("Dispatch wayland event");
					let _span = _span.enter();
					let _ = display.dispatch_clients(&mut *state.lock());
					let _ = display.flush_clients();
				}
				_ = flush_notify.notified() => {
					drop(async_fd);
					let _ = display.flush_clients();
				},
			}
		}
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

		self.flush_notify.notify_waiters();
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
		self.client_listener.abort();
		self.client_dispatcher.abort();
	}
}

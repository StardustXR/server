pub mod compositor;
pub mod panel_item;
pub mod seat;
pub mod shaders;
pub mod state;
pub mod surface;
pub mod xdg_decoration;
pub mod xdg_shell;

use self::{panel_item::PanelItem, state::WaylandState};
use crate::{nodes::core::Node, wayland::state::ClientState};
use anyhow::{ensure, Result};
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use slog::Logger;
use smithay::{
	backend::{
		egl::EGLContext,
		renderer::{
			gles2::Gles2Renderer,
			utils::{import_surface_tree, on_commit_buffer_handler, RendererSurfaceStateUserData},
		},
	},
	desktop::utils::send_frames_surface_tree,
	reexports::wayland_server::{backend::GlobalId, Display, ListeningSocket},
	wayland::{compositor::with_states, shell::xdg::XdgToplevelSurfaceData},
};
use std::os::unix::prelude::AsRawFd;
use std::{
	ffi::c_void,
	os::unix::{
		net::UnixListener,
		prelude::{FromRawFd, RawFd},
	},
	sync::Arc,
};
use stereokit as sk;
use stereokit::StereoKit;
use surface::CoreSurface;
use tokio::{
	io::unix::AsyncFd, net::UnixListener as AsyncUnixListener, sync::mpsc, task::JoinHandle,
};

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

pub struct Wayland {
	log: slog::Logger,

	display: Arc<Mutex<Display<WaylandState>>>,
	join_handle: JoinHandle<Result<()>>,
	renderer: Gles2Renderer,
	state: Arc<Mutex<WaylandState>>,
}
impl Wayland {
	pub fn new(log: Logger) -> Result<Self> {
		let egl_raw_handles = get_sk_egl()?;
		let renderer = unsafe {
			Gles2Renderer::new(
				EGLContext::from_raw(
					egl_raw_handles.display,
					egl_raw_handles.config,
					egl_raw_handles.context,
					log.clone(),
				)?,
				log.clone(),
			)?
		};

		let display: Display<WaylandState> = Display::new()?;
		let display_handle = display.handle();

		let display = Arc::new(Mutex::new(display));
		let state = Arc::new(Mutex::new(WaylandState::new(
			log.clone(),
			display_handle.clone(),
		)));

		let (global_destroy_queue_in, global_destroy_queue) = mpsc::channel(8);
		GLOBAL_DESTROY_QUEUE.set(global_destroy_queue_in).unwrap();

		let join_handle =
			Wayland::start_loop(display.clone(), state.clone(), global_destroy_queue)?;

		Ok(Wayland {
			log,
			display,
			join_handle,
			renderer,
			state,
		})
	}

	fn start_loop(
		display: Arc<Mutex<Display<WaylandState>>>,
		state: Arc<Mutex<WaylandState>>,
		mut global_destroy_queue: mpsc::Receiver<GlobalId>,
	) -> Result<JoinHandle<Result<()>>> {
		let socket = ListeningSocket::bind_auto("wayland", 0..33)?;
		if let Some(socket_name) = socket.socket_name() {
			println!("Wayland compositor {:?} active", socket_name);
		}

		let listen_async =
			AsyncUnixListener::from_std(unsafe { UnixListener::from_raw_fd(socket.as_raw_fd()) })?;

		let dispatch_poll_fd: RawFd = display.lock().backend().poll_fd();
		let dispatch_poll_listener = AsyncFd::new(dispatch_poll_fd)?;

		let dh1 = display.lock().handle();
		let mut dh2 = dh1.clone();

		Ok(tokio::task::spawn(async move {
			let _socket = socket; // Keep the socket alive
			loop {
				tokio::select! {
					e = global_destroy_queue.recv() => { // New global to destroy
						dh1.remove_global::<WaylandState>(e.unwrap());
					}
					acc = listen_async.accept() => { // New client connected
						let (stream, _) = acc?;
						dh2.insert_client(stream.into_std()?, Arc::new(ClientState))?;
					}
					e = dispatch_poll_listener.readable() => { // Dispatch
						let mut guard = e?;
						let mut display = display.lock();
						display.dispatch_clients(&mut *state.lock())?;
						display.flush_clients()?;
						guard.clear_ready();
					}
				}
			}
		}))
	}

	pub fn frame(&mut self, sk: &StereoKit) {
		let log = self.log.clone();
		let time_ms = (sk.time_getf() * 1000.) as u32;
		let toplevel_surfaces = self
			.state
			.lock()
			.xdg_shell_state
			.toplevel_surfaces(|surfs| surfs.to_vec());
		for surf in toplevel_surfaces {
			// Let Smithay handle all the buffer maintenance
			on_commit_buffer_handler(surf.wl_surface());
			// Import all surface buffers into textures
			import_surface_tree(&mut self.renderer, surf.wl_surface(), &log).unwrap();

			with_states(surf.wl_surface(), |data| {
				let mapped = data
					.data_map
					.get::<RendererSurfaceStateUserData>()
					.map(|surface_states| surface_states.borrow().wl_buffer().is_some())
					.unwrap_or(false);

				if mapped && data.data_map.get::<XdgToplevelSurfaceData>().is_some() {
					data.data_map.insert_if_missing_threadsafe(CoreSurface::new);
					data.data_map.insert_if_missing_threadsafe(|| {
						PanelItem::create(
							&self.display,
							self.display.lock().handle(),
							&data.data_map,
							surf.wl_surface().clone(),
						)
					});

					if let Some(core_surface) = data.data_map.get::<CoreSurface>() {
						core_surface.update_tex(sk, data, &self.renderer);
						if let Some(panel_item) = data.data_map.get::<Arc<Node>>() {
							PanelItem::apply_surface_materials(panel_item, core_surface);
						}
					}
				}
			});
			send_frames_surface_tree(surf.wl_surface(), time_ms);
		}
		self.display.lock().flush_clients().unwrap();
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

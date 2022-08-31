pub mod compositor;
pub mod panel_item;
pub mod seat;
pub mod shaders;
pub mod surface;
pub mod xdg_decoration;
pub mod xdg_shell;
use std::{ffi::c_void, sync::Arc};

use anyhow::{ensure, Result};
use parking_lot::Mutex;
use slog::Logger;
use smithay::{
	backend::{egl::EGLContext, renderer::gles2::Gles2Renderer},
	delegate_output, delegate_shm,
	desktop::utils::send_frames_surface_tree,
	reexports::wayland_server::{
		backend::{ClientData, ClientId, DisconnectReason},
		protocol::wl_output::Subpixel,
		Display, DisplayHandle, ListeningSocket,
	},
	utils::Size,
	wayland::{
		buffer::BufferHandler,
		compositor::{with_states, CompositorState},
		output::{Output, OutputManagerState, Scale::Integer},
		shell::xdg::{decoration::XdgDecorationState, XdgShellState},
		shm::{ShmHandler, ShmState},
	},
};
use stereokit as sk;
use stereokit::StereoKit;
use surface::CoreSurface;

use self::seat::SeatDelegate;

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

pub struct ClientState;
impl ClientData for ClientState {
	fn initialized(&self, client_id: ClientId) {
		println!("Wayland client {:?} connected", client_id);
	}

	fn disconnected(&self, client_id: ClientId, reason: DisconnectReason) {
		println!(
			"Wayland client {:?} disconnected because {:#?}",
			client_id, reason
		);
	}
}

pub struct WaylandState {
	pub log: slog::Logger,

	pub display: Arc<Mutex<Display<WaylandState>>>,
	pub display_handle: DisplayHandle,
	pub socket: ListeningSocket,
	pub renderer: Gles2Renderer,
	pub compositor_state: CompositorState,
	pub xdg_shell_state: XdgShellState,
	pub xdg_decoration_state: XdgDecorationState,
	pub shm_state: ShmState,
	pub output_manager_state: OutputManagerState,
	pub output: Output,
	pub seat_state: SeatDelegate,
	// pub data_device_state: DataDeviceState,
}

impl WaylandState {
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
		let socket = ListeningSocket::bind_auto("wayland", 0..33)?;
		if let Some(socket_name) = socket.socket_name() {
			println!("Wayland compositor {:?} active", socket_name);
		}
		let display_handle = display.handle();

		let compositor_state = CompositorState::new::<Self, _>(&display_handle, log.clone());
		let xdg_shell_state = XdgShellState::new::<Self, _>(&display_handle, log.clone());
		let xdg_decoration_state = XdgDecorationState::new::<Self, _>(&display_handle, log.clone());
		let shm_state = ShmState::new::<Self, _>(&display_handle, vec![], log.clone());
		let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&display_handle);
		let output = Output::new(
			"1x".to_owned(),
			smithay::wayland::output::PhysicalProperties {
				size: Size::default(),
				subpixel: Subpixel::None,
				make: "Virtual XR Display".to_owned(),
				model: "Your Headset Name Here".to_owned(),
			},
			log.clone(),
		);
		let _global = output.create_global::<Self>(&display_handle);
		output.change_current_state(None, None, Some(Integer(2)), None);
		// let data_device_state = DataDeviceState::new(&dh, log.clone());

		println!("Init Wayland compositor");
		Ok(WaylandState {
			log,
			display: Arc::new(Mutex::new(display)),
			display_handle,
			socket,
			renderer,
			compositor_state,
			xdg_shell_state,
			xdg_decoration_state,
			shm_state,
			output_manager_state,
			output,
			seat_state: SeatDelegate,
			// data_device_state,
		})
	}

	pub fn frame(&mut self, sk: &StereoKit) {
		let display_clone = self.display.clone();
		let mut display = display_clone.lock();
		if let Ok(Some(client)) = self.socket.accept() {
			let _ = display
				.handle()
				.insert_client(client, Arc::new(ClientState));
		}
		display.dispatch_clients(self).unwrap();
		display.flush_clients().unwrap();

		drop(display);
		drop(display_clone);

		let time_ms = (sk.time_getf() * 1000.) as u32;
		self.xdg_shell_state.toplevel_surfaces(|surfs| {
			for surf in surfs.iter() {
				with_states(surf.wl_surface(), |data| {
					if let Some(core_surface) = data.data_map.get::<CoreSurface>() {
						core_surface.update_tex(sk);
					}
				});
				send_frames_surface_tree(surf.wl_surface(), time_ms);
			}
		});
	}
}
impl Drop for WaylandState {
	fn drop(&mut self) {
		println!("Cleanly shut down the Wayland compositor");
	}
}
impl BufferHandler for WaylandState {
	fn buffer_destroyed(
		&mut self,
		_buffer: &smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer,
	) {
	}
}
impl ShmHandler for WaylandState {
	fn shm_state(&self) -> &smithay::wayland::shm::ShmState {
		&self.shm_state
	}
}
delegate_shm!(WaylandState);
delegate_output!(WaylandState);

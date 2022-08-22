pub mod compositor;
pub mod xdg_decoration;
mod xdg_shell;
use anyhow::Result;
use slog::Logger;
use smithay::{
	backend::renderer::gles2::Gles2Renderer,
	delegate_output, delegate_shm,
	reexports::wayland_server::{
		backend::{ClientData, ClientId, DisconnectReason},
		protocol::wl_output::Subpixel,
		Display, DisplayHandle,
	},
	utils::Size,
	wayland::{
		buffer::BufferHandler,
		compositor::CompositorState,
		output::{Output, OutputManagerState, Scale::Integer},
		seat::SeatState,
		shell::xdg::{decoration::XdgDecorationState, XdgShellState},
		shm::{ShmHandler, ShmState},
	},
};

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

	pub display_handle: DisplayHandle,
	pub renderer: Gles2Renderer,
	pub compositor_state: CompositorState,
	pub xdg_shell_state: XdgShellState,
	pub xdg_decoration_state: XdgDecorationState,
	pub shm_state: ShmState,
	pub output_manager_state: OutputManagerState,
	pub output: Output,
	pub seat_state: SeatState<WaylandState>,
	// pub data_device_state: DataDeviceState,
}

impl WaylandState {
	pub fn new(
		display: &Display<WaylandState>,
		renderer: Gles2Renderer,
		log: Logger,
	) -> Result<Self> {
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
		let seat_state = SeatState::new();
		// let data_device_state = DataDeviceState::new(&dh, log.clone());

		Ok(WaylandState {
			log,
			display_handle,
			renderer,
			compositor_state,
			xdg_shell_state,
			xdg_decoration_state,
			shm_state,
			output_manager_state,
			output,
			seat_state,
			// data_device_state,
		})
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

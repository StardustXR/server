use std::sync::Arc;

use parking_lot::Mutex;
use slog::Logger;
use smithay::{
	delegate_output, delegate_shm,
	output::{Output, Scale, Subpixel},
	reexports::wayland_server::{
		backend::{ClientData, ClientId, DisconnectReason},
		Display, DisplayHandle,
	},
	utils::Size,
	wayland::{
		buffer::BufferHandler,
		compositor::CompositorState,
		output::OutputManagerState,
		shell::xdg::{decoration::XdgDecorationState, XdgShellState},
		shm::{ShmHandler, ShmState},
	},
};

use super::seat::SeatDelegate;

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
	pub display: Arc<Mutex<Display<WaylandState>>>,
	pub display_handle: DisplayHandle,

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
	pub fn new(
		log: Logger,
		display: Arc<Mutex<Display<WaylandState>>>,
		display_handle: DisplayHandle,
	) -> Self {
		let compositor_state = CompositorState::new::<Self, _>(&display_handle, log.clone());
		let xdg_shell_state = XdgShellState::new::<Self, _>(&display_handle, log.clone());
		let xdg_decoration_state = XdgDecorationState::new::<Self, _>(&display_handle, log.clone());
		let shm_state = ShmState::new::<Self, _>(&display_handle, vec![], log.clone());
		let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&display_handle);
		let output = Output::new(
			"1x".to_owned(),
			smithay::output::PhysicalProperties {
				size: Size::default(),
				subpixel: Subpixel::None,
				make: "Virtual XR Display".to_owned(),
				model: "Your Headset Name Here".to_owned(),
			},
			log.clone(),
		);
		let _global = output.create_global::<Self>(&display_handle);
		output.change_current_state(None, None, Some(Scale::Integer(2)), None);
		// let data_device_state = DataDeviceState::new(&dh, log.clone());

		println!("Init Wayland compositor");

		WaylandState {
			display,
			display_handle,

			compositor_state,
			xdg_shell_state,
			xdg_decoration_state,
			shm_state,
			output_manager_state,
			output,
			seat_state: SeatDelegate,
			// data_device_state,
		}
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

use crate::wayland::seat::SeatData;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use smithay::{
	backend::{
		allocator::dmabuf::Dmabuf,
		renderer::{gles::GlesRenderer, ImportDma},
	},
	delegate_dmabuf, delegate_output, delegate_shm,
	output::{Mode, Output, Scale, Subpixel},
	reexports::{
		wayland_protocols::xdg::{
			decoration::zv1::server::zxdg_decoration_manager_v1::ZxdgDecorationManagerV1,
			shell::server::xdg_wm_base::XdgWmBase,
		},
		wayland_protocols_misc::server_decoration::server::org_kde_kwin_server_decoration_manager::Mode as DecorationMode,
		wayland_server::{
			backend::{ClientData, ClientId, DisconnectReason},
			protocol::{wl_buffer::WlBuffer, wl_data_device_manager::WlDataDeviceManager},
			Display, DisplayHandle,
		},
	},
	utils::{Size, Transform},
	wayland::{
		buffer::BufferHandler,
		compositor::{CompositorClientState, CompositorState},
		dmabuf::{self, DmabufGlobal, DmabufHandler, DmabufState},
		shell::kde::decoration::KdeDecorationState,
		shm::{ShmHandler, ShmState},
	},
};
use std::sync::{Arc, Weak};
use tracing::info;

#[derive(Default)]
pub struct ClientState {
	pub compositor_state: CompositorClientState,
}
impl ClientData for ClientState {
	fn initialized(&self, client_id: ClientId) {
		info!("Wayland client {:?} connected", client_id);
	}

	fn disconnected(&self, client_id: ClientId, reason: DisconnectReason) {
		info!(
			"Wayland client {:?} disconnected because {:#?}",
			client_id, reason
		);
	}
}

pub struct WaylandState {
	pub weak_ref: Weak<Mutex<WaylandState>>,
	pub display: Arc<Mutex<Display<WaylandState>>>,
	pub display_handle: DisplayHandle,

	pub compositor_state: CompositorState,
	// pub xdg_activation_state: XdgActivationState,
	pub kde_decoration_state: KdeDecorationState,
	pub shm_state: ShmState,
	pub dmabuf_state: DmabufState,
	pub dmabuf_global: DmabufGlobal,
	pub output: Output,
	pub seats: FxHashMap<ClientId, Arc<SeatData>>,
}

impl WaylandState {
	pub fn new(
		display: Arc<Mutex<Display<WaylandState>>>,
		display_handle: DisplayHandle,
		renderer: &GlesRenderer,
	) -> Arc<Mutex<Self>> {
		let compositor_state = CompositorState::new::<Self>(&display_handle);
		// let xdg_activation_state = XdgActivationState::new::<Self, _>(&display_handle);
		let kde_decoration_state =
			KdeDecorationState::new::<Self>(&display_handle, DecorationMode::Server);
		let shm_state = ShmState::new::<Self>(&display_handle, vec![]);
		let mut dmabuf_state = DmabufState::new();
		let dmabuf_global = dmabuf_state.create_global::<Self>(
			&display_handle,
			renderer.dmabuf_formats().collect::<Vec<_>>(),
		);
		let output = Output::new(
			"1x".to_owned(),
			smithay::output::PhysicalProperties {
				size: Size::default(),
				subpixel: Subpixel::None,
				make: "Virtual XR Display".to_owned(),
				model: "Your Headset Name Here".to_owned(),
			},
		);
		let _output_global = output.create_global::<Self>(&display_handle);
		let mode = Mode {
			size: (4096, 4096).into(),
			refresh: 60000,
		};
		output.change_current_state(
			Some(mode),
			Some(Transform::Normal),
			Some(Scale::Integer(2)),
			None,
		);
		output.set_preferred(mode);
		display_handle.create_global::<Self, WlDataDeviceManager, _>(3, ());
		display_handle.create_global::<Self, XdgWmBase, _>(5, ());
		display_handle.create_global::<Self, ZxdgDecorationManagerV1, _>(1, ());

		info!("Init Wayland compositor");

		Arc::new_cyclic(|weak| {
			Mutex::new(WaylandState {
				weak_ref: weak.clone(),
				display,
				display_handle,

				compositor_state,
				// xdg_activation_state,
				kde_decoration_state,
				shm_state,
				dmabuf_state,
				dmabuf_global,
				output,
				seats: FxHashMap::default(),
			})
		})
	}

	pub fn new_client(&mut self, client: ClientId, dh: &DisplayHandle) {
		let seat_data = SeatData::new(dh, client.clone());
		self.seats.insert(client, seat_data);
	}
}
impl Drop for WaylandState {
	fn drop(&mut self) {
		info!("Cleanly shut down the Wayland compositor");
	}
}
impl BufferHandler for WaylandState {
	fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}
impl ShmHandler for WaylandState {
	fn shm_state(&self) -> &ShmState {
		&self.shm_state
	}
}
impl DmabufHandler for WaylandState {
	fn dmabuf_state(&mut self) -> &mut DmabufState {
		&mut self.dmabuf_state
	}

	fn dmabuf_imported(
		&mut self,
		_global: &DmabufGlobal,
		_dmabuf: Dmabuf,
	) -> Result<(), dmabuf::ImportError> {
		Ok(())
	}
}
delegate_dmabuf!(WaylandState);
delegate_shm!(WaylandState);
delegate_output!(WaylandState);

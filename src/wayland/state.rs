use super::seat::SeatWrapper;
use crate::wayland::drm::wl_drm::WlDrm;
use parking_lot::Mutex;
use smithay::{
	backend::{
		allocator::{Fourcc, dmabuf::Dmabuf},
		egl::EGLDevice,
		renderer::gles::GlesRenderer,
	},
	delegate_dmabuf, delegate_output, delegate_shm,
	desktop::PopupManager,
	input::{SeatState, keyboard::XkbConfig},
	output::{Mode, Output, Scale, Subpixel},
	reexports::{
		wayland_protocols::xdg::{
			decoration::zv1::server::zxdg_decoration_manager_v1::ZxdgDecorationManagerV1,
			shell::server::xdg_toplevel::WmCapabilities,
		},
		wayland_protocols_misc::server_decoration::server::org_kde_kwin_server_decoration_manager::Mode as DecorationMode,
		wayland_server::{
			DisplayHandle,
			backend::{ClientData, ClientId, DisconnectReason},
			protocol::{
				wl_buffer::WlBuffer, wl_data_device_manager::WlDataDeviceManager,
				wl_output::WlOutput,
			},
		},
	},
	utils::{Size, Transform},
	wayland::{
		buffer::BufferHandler,
		compositor::{CompositorClientState, CompositorState},
		dmabuf::{
			self, DmabufFeedback, DmabufFeedbackBuilder, DmabufGlobal, DmabufHandler, DmabufState,
		},
		output::OutputHandler,
		shell::{
			kde::decoration::KdeDecorationState,
			xdg::{WmCapabilitySet, XdgShellState},
		},
		shm::{ShmHandler, ShmState},
	},
};
use std::sync::{Arc, OnceLock};
use tokio::sync::mpsc::UnboundedSender;
use tracing::{info, warn};

pub struct ClientState {
	pub pid: Option<i32>,
	pub id: OnceLock<ClientId>,
	pub compositor_state: CompositorClientState,
	pub seat: Arc<SeatWrapper>,
}
impl ClientData for ClientState {
	fn initialized(&self, client_id: ClientId) {
		info!("Wayland client {:?} connected", client_id);
		let _ = self.id.set(client_id);
	}

	fn disconnected(&self, client_id: ClientId, reason: DisconnectReason) {
		info!(
			"Wayland client {:?} disconnected because {:#?}",
			client_id, reason
		);
	}
}

pub struct WaylandState {
	pub compositor_state: CompositorState,
	// pub xdg_activation_state: XdgActivationState,
	pub kde_decoration_state: KdeDecorationState,
	pub shm_state: ShmState,
	dmabuf_state: (DmabufState, DmabufGlobal, Option<DmabufFeedback>),
	pub drm_formats: Vec<Fourcc>,
	pub dmabuf_tx: UnboundedSender<(Dmabuf, Option<dmabuf::ImportNotifier>)>,
	pub seat_state: SeatState<Self>,
	pub seat: Arc<SeatWrapper>,
	pub xdg_shell: XdgShellState,
	pub popup_manager: PopupManager,
	pub output: Output,
}

impl WaylandState {
	pub fn new(
		display_handle: DisplayHandle,
		renderer: &GlesRenderer,
		dmabuf_tx: UnboundedSender<(Dmabuf, Option<dmabuf::ImportNotifier>)>,
	) -> Arc<Mutex<Self>> {
		let compositor_state = CompositorState::new::<Self>(&display_handle);
		// let xdg_activation_state = XdgActivationState::new::<Self, _>(&display_handle);
		let kde_decoration_state =
			KdeDecorationState::new::<Self>(&display_handle, DecorationMode::Server);
		let shm_state = ShmState::new::<Self>(&display_handle, vec![]);
		let render_node = EGLDevice::device_for_display(renderer.egl_context().display())
			.and_then(|device| device.try_get_render_node());
		let dmabuf_formats = renderer
			.egl_context()
			.dmabuf_render_formats()
			.iter()
			.cloned()
			.collect::<Vec<_>>();
		let drm_formats = dmabuf_formats.iter().map(|f| f.code).collect();

		let dmabuf_default_feedback = match render_node {
			Ok(Some(node)) => DmabufFeedbackBuilder::new(node.dev_id(), dmabuf_formats.clone())
				.build()
				.ok(),
			Ok(None) => {
				warn!("failed to query render node, dmabuf will use v3");
				None
			}
			Err(err) => {
				warn!(?err, "failed to egl device for display, dmabuf will use v3");
				None
			}
		};
		// if we failed to build dmabuf feedback we fall back to dmabuf v3
		// Note: egl on Mesa requires either v4 or wl_drm (initialized with bind_wl_display)
		let dmabuf_state = if let Some(default_feedback) = dmabuf_default_feedback {
			let mut dmabuf_state = DmabufState::new();
			let dmabuf_global = dmabuf_state.create_global_with_default_feedback::<WaylandState>(
				&display_handle,
				&default_feedback,
			);
			(dmabuf_state, dmabuf_global, Some(default_feedback))
		} else {
			let mut dmabuf_state = DmabufState::new();
			let dmabuf_global =
				dmabuf_state.create_global::<WaylandState>(&display_handle, dmabuf_formats.clone());
			(dmabuf_state, dmabuf_global, None)
		};

		let mut seat_state = SeatState::new();
		let mut seat = seat_state.new_wl_seat(&display_handle, "seat0");
		seat.add_pointer();
		seat.add_keyboard(XkbConfig::default(), 200, 25).unwrap();
		seat.add_touch();

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
			size: (1024, 1024).into(),
			refresh: 60000,
		};
		output.change_current_state(
			Some(mode),
			Some(Transform::Normal),
			Some(Scale::Integer(2)),
			None,
		);
		output.set_preferred(mode);

		let mut xdg_shell = XdgShellState::new::<Self>(&display_handle);
		let popup_manager = PopupManager::default();
		let mut capabilities = WmCapabilitySet::default();
		capabilities.set(WmCapabilities::Maximize);
		capabilities.set(WmCapabilities::Fullscreen);
		capabilities.unset(WmCapabilities::Minimize);
		capabilities.unset(WmCapabilities::WindowMenu);
		xdg_shell.replace_capabilities(capabilities);
		display_handle.create_global::<Self, WlDataDeviceManager, _>(3, ());
		display_handle.create_global::<Self, ZxdgDecorationManagerV1, _>(1, ());
		display_handle.create_global::<Self, WlDrm, _>(2, ());

		info!("Init Wayland compositor");

		Arc::new_cyclic(|weak| {
			Mutex::new(WaylandState {
				compositor_state,
				// xdg_activation_state,
				kde_decoration_state,
				shm_state,
				drm_formats,
				dmabuf_state,
				dmabuf_tx,
				seat_state,
				seat: Arc::new(SeatWrapper::new(weak.clone(), seat)),
				xdg_shell,
				popup_manager,
				output,
			})
		})
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
		&mut self.dmabuf_state.0
	}

	fn dmabuf_imported(
		&mut self,
		_global: &DmabufGlobal,
		dmabuf: Dmabuf,
		notifier: dmabuf::ImportNotifier,
	) {
		self.dmabuf_tx.send((dmabuf, Some(notifier))).unwrap();
	}
}
impl OutputHandler for WaylandState {
	fn output_bound(&mut self, _output: Output, _wl_output: WlOutput) {}
}
delegate_dmabuf!(WaylandState);
delegate_shm!(WaylandState);
delegate_output!(WaylandState);

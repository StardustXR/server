use crate::wayland::relative_pointer::RelativePointerManager;
use crate::wayland::{Client, WaylandResult};
use crate::wayland::{
	WaylandError,
	core::{
		compositor::{Compositor, WlCompositor},
		data_device::DataDeviceManager,
		output::{Output, WlOutput},
		seat::{Seat, WlSeat},
		shm::{Shm, WlShm},
		subcompositor::Subcompositor,
	},
	dmabuf::Dmabuf,
	mesa_drm::MesaDrm,
	presentation::Presentation,
	util::ClientExt,
	viewporter::Viewporter,
	xdg::wm_base::{WmBase, XdgWmBase},
};
use waynest::{NewId, ObjectId};
use waynest_protocols::server::{
	core::wayland::{
		wl_data_device_manager::WlDataDeviceManager, wl_registry::*,
		wl_subcompositor::WlSubcompositor,
	},
	mesa::drm::wl_drm::WlDrm,
	stable::{
		linux_dmabuf_v1::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
		presentation_time::wp_presentation::WpPresentation,
		viewporter::wp_viewporter::WpViewporter,
	},
	unstable::relative_pointer_unstable_v1::zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1,
};
use waynest_server::Client as _;

struct RegistryGlobals;
impl RegistryGlobals {
	pub const COMPOSITOR: u32 = 0;
	pub const SHM: u32 = 1;
	pub const WM_BASE: u32 = 2;
	pub const SEAT: u32 = 3;
	pub const DATA_DEVICE_MANAGER: u32 = 4;
	pub const OUTPUT: u32 = 5;
	pub const DMABUF: u32 = 6;
	pub const WL_DRM: u32 = 7;
	pub const PRESENTATION: u32 = 8;
	pub const VIEWPORTER: u32 = 9;
	pub const RELATIVE_POINTER: u32 = 10;
	pub const SUBCOMPOSITOR: u32 = 11;
}

#[derive(Debug, waynest_server::RequestDispatcher, Default)]
#[waynest(error = crate::wayland::WaylandError, connection = crate::wayland::Client)]
pub struct Registry;

impl Registry {
	pub async fn advertise_globals(
		&self,
		client: &mut Client,
		sender_id: ObjectId,
	) -> WaylandResult<()> {
		self.global(
			client,
			sender_id,
			RegistryGlobals::COMPOSITOR,
			Compositor::INTERFACE.to_string(),
			Compositor::VERSION,
		)
		.await?;

		self.global(
			client,
			sender_id,
			RegistryGlobals::SHM,
			Shm::INTERFACE.to_string(),
			Shm::VERSION,
		)
		.await?;

		self.global(
			client,
			sender_id,
			RegistryGlobals::WM_BASE,
			WmBase::INTERFACE.to_string(),
			WmBase::VERSION,
		)
		.await?;

		self.global(
			client,
			sender_id,
			RegistryGlobals::SEAT,
			Seat::INTERFACE.to_string(),
			Seat::VERSION,
		)
		.await?;

		self.global(
			client,
			sender_id,
			RegistryGlobals::DATA_DEVICE_MANAGER,
			DataDeviceManager::INTERFACE.to_string(),
			DataDeviceManager::VERSION,
		)
		.await?;

		self.global(
			client,
			sender_id,
			RegistryGlobals::OUTPUT,
			Output::INTERFACE.to_string(),
			Output::VERSION,
		)
		.await?;

		self.global(
			client,
			sender_id,
			RegistryGlobals::DMABUF,
			crate::wayland::dmabuf::Dmabuf::INTERFACE.to_string(),
			Dmabuf::VERSION,
		)
		.await?;

		self.global(
			client,
			sender_id,
			RegistryGlobals::WL_DRM,
			crate::wayland::mesa_drm::MesaDrm::INTERFACE.to_string(),
			MesaDrm::VERSION,
		)
		.await?;

		self.global(
			client,
			sender_id,
			RegistryGlobals::PRESENTATION,
			Presentation::INTERFACE.to_string(),
			Presentation::VERSION,
		)
		.await?;

		self.global(
			client,
			sender_id,
			RegistryGlobals::VIEWPORTER,
			Viewporter::INTERFACE.to_string(),
			Viewporter::VERSION,
		)
		.await?;

		self.global(
			client,
			sender_id,
			RegistryGlobals::RELATIVE_POINTER,
			RelativePointerManager::INTERFACE.to_string(),
			RelativePointerManager::VERSION,
		)
		.await?;

		self.global(
			client,
			sender_id,
			RegistryGlobals::SUBCOMPOSITOR,
			Subcompositor::INTERFACE.to_string(),
			Subcompositor::VERSION,
		)
		.await?;

		Ok(())
	}
}

impl WlRegistry for Registry {
	type Connection = crate::wayland::Client;

	async fn bind(
		&self,
		client: &mut Self::Connection,
		_sender_id: ObjectId,
		name: u32,
		new_id: NewId,
	) -> WaylandResult<()> {
		match name {
			RegistryGlobals::COMPOSITOR => {
				tracing::info!("Binding compositor");
				client.insert(new_id.object_id, Compositor)?;
			}
			RegistryGlobals::SHM => {
				tracing::info!("Binding SHM");
				let shm = client.insert(new_id.object_id, Shm)?;
				shm.advertise_formats(client, new_id.object_id).await?;
			}
			RegistryGlobals::WM_BASE => {
				tracing::info!("Binding WM_BASE");
				client.insert(
					new_id.object_id,
					WmBase::new(new_id.object_id, new_id.version),
				)?;
			}
			RegistryGlobals::SEAT => {
				tracing::info!("Binding seat with id {}", new_id.object_id);
				let seat = Seat::new(client, new_id.object_id, new_id.version).await?;
				let seat = client.insert(new_id.object_id, seat)?;
				let _ = client.display().seat.set(seat.clone());

				tracing::info!("Seat capabilities advertised");
			}
			RegistryGlobals::DATA_DEVICE_MANAGER => {
				tracing::info!("Binding data device manager");
				client.insert(new_id.object_id, DataDeviceManager)?;
			}
			RegistryGlobals::OUTPUT => {
				tracing::info!("Binding output");
				let output = client.insert(
					new_id.object_id,
					Output {
						id: new_id.object_id,
						version: new_id.version,
					},
				)?;
				let _ = client.display().output.set(output.clone());
				output.advertise_outputs(client).await?;
			}
			RegistryGlobals::DMABUF => {
				tracing::info!("Binding dmabuf");

				let dmabuf = Dmabuf::new(client, new_id.object_id, new_id.version).await?;
				client.insert(new_id.object_id, dmabuf)?;
			}
			RegistryGlobals::WL_DRM => {
				tracing::info!("Binding wl_drm");

				let drm = MesaDrm::new(client, new_id.object_id, new_id.version).await?;
				client.insert(new_id.object_id, drm)?;
			}
			RegistryGlobals::PRESENTATION => {
				tracing::info!("Binding wp_presentation");

				client
					.insert(new_id.object_id, Presentation::new(new_id.object_id))?
					.clock_id(
						client,
						new_id.object_id,
						rustix::time::ClockId::Monotonic as u32,
					)
					.await?;
			}
			RegistryGlobals::VIEWPORTER => {
				tracing::info!("Binding wp_viewporter");

				client.insert(new_id.object_id, Viewporter::new(new_id.object_id))?;
			}
			RegistryGlobals::RELATIVE_POINTER => {
				tracing::info!("Binding zwp_relative_pointer_manager_v1");

				client.insert(new_id.object_id, RelativePointerManager(new_id.object_id))?;
			}
			RegistryGlobals::SUBCOMPOSITOR => {
				tracing::info!("Binding wl_subcompositor");

				client.insert(new_id.object_id, Subcompositor)?;
			}
			id => {
				tracing::error!(id, "Wayland: failed to bind to registry global");
				return Err(WaylandError::UnknownGlobal(name));
			}
		}

		Ok(())
	}
}

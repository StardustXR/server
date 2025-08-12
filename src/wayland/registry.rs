use crate::wayland::{
	core::{
		compositor::{Compositor, WlCompositor},
		data_device::DataDeviceManager,
		output::{Output, WlOutput},
		seat::{Seat, WlSeat},
		shm::{Shm, WlShm},
	},
	dmabuf::Dmabuf,
	mesa_drm::MesaDrm,
	presentation::Presentation,
	util::ClientExt,
	xdg::wm_base::{WmBase, XdgWmBase},
};
use waynest::{
	server::{
		Client, Dispatcher, Error, Result,
		protocol::{
			core::wayland::{wl_data_device_manager::WlDataDeviceManager, wl_registry::*},
			external::drm::wl_drm::WlDrm,
			stable::{
				linux_dmabuf_v1::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
				presentation_time::wp_presentation::WpPresentation,
			},
		},
	},
	wire::{NewId, ObjectId},
};

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
}

#[derive(Debug, Dispatcher, Default)]
pub struct Registry;

impl Registry {
	pub async fn advertise_globals(&self, client: &mut Client, sender_id: ObjectId) -> Result<()> {
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

		Ok(())
	}
}

impl WlRegistry for Registry {
	async fn bind(
		&self,
		client: &mut Client,
		_sender_id: ObjectId,
		name: u32,
		new_id: NewId,
	) -> Result<()> {
		match name {
			RegistryGlobals::COMPOSITOR => {
				tracing::info!("Binding compositor");
				client.insert(new_id.object_id, Compositor);
			}
			RegistryGlobals::SHM => {
				tracing::info!("Binding SHM");
				let shm = client.insert(new_id.object_id, Shm);
				shm.advertise_formats(client, new_id.object_id).await?;
			}
			RegistryGlobals::WM_BASE => {
				tracing::info!("Binding WM_BASE");
				client.insert(new_id.object_id, WmBase);
			}
			RegistryGlobals::SEAT => {
				tracing::info!("Binding seat with id {}", new_id.object_id);
				let seat = Seat::new(client, new_id.object_id, new_id.version).await?;
				let seat = client.insert(new_id.object_id, seat);
				let _ = client.display().seat.set(seat.clone());

				tracing::info!("Seat capabilities advertised");
			}
			RegistryGlobals::DATA_DEVICE_MANAGER => {
				tracing::info!("Binding data device manager");
				client.insert(new_id.object_id, DataDeviceManager);
			}
			RegistryGlobals::OUTPUT => {
				tracing::info!("Binding output");
				let output = client.insert(
					new_id.object_id,
					Output {
						id: new_id.object_id,
						version: new_id.version,
					},
				);
				let _ = client.display().output.set(output.clone());
				output.advertise_outputs(client).await?;
			}
			RegistryGlobals::DMABUF => {
				tracing::info!("Binding dmabuf");

				let dmabuf = Dmabuf::new(client, new_id.object_id, new_id.version).await?;
				client.insert(new_id.object_id, dmabuf);
			}
			RegistryGlobals::WL_DRM => {
				tracing::info!("Binding wl_drm");

				let drm = MesaDrm::new(client, new_id.object_id, new_id.version).await?;
				client.insert(new_id.object_id, drm);
			}
			RegistryGlobals::PRESENTATION => {
				tracing::info!("Binding wp_presentation");

				client.insert(new_id.object_id, Presentation);
			}
			id => {
				tracing::error!(id, "Wayland: failed to bind to registry global");
				return Err(Error::MissingObject(unsafe { ObjectId::from_raw(name) }));
			}
		}

		Ok(())
	}
}

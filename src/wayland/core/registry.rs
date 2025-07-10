use crate::wayland::{
	core::{
		compositor::{Compositor, WlCompositor},
		display::Display,
		output::{Output, WlOutput},
		seat::{Seat, WlSeat},
		shm::{Shm, WlShm},
	},
	dmabuf::Dmabuf,
	xdg::wm_base::{WmBase, XdgWmBase},
};
pub use waynest::server::protocol::core::wayland::wl_registry::*;

use waynest::server::protocol::stable::linux_dmabuf_v1::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1;
use waynest::{
	server::{Client, Dispatcher, Error, Result},
	wire::{NewId, ObjectId},
};

struct RegistryGlobals;
impl RegistryGlobals {
	pub const COMPOSITOR: u32 = 0;
	pub const SHM: u32 = 1;
	pub const WM_BASE: u32 = 2;
	pub const SEAT: u32 = 3;
	pub const OUTPUT: u32 = 4;
	pub const DMABUF: u32 = 5;
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
				let seat = client.insert(new_id.object_id, Seat::new());
				if let Some(display) = client.get::<Display>(ObjectId::DISPLAY) {
					tracing::info!("Setting seat in display");
					let _ = display.seat.set(seat.clone());
					tracing::info!("Seat set successfully");
				} else {
					tracing::warn!("No display found to set seat");
				}
				seat.advertise_capabilities(client, new_id.object_id)
					.await?;
				tracing::info!("Seat capabilities advertised");
			}
			RegistryGlobals::OUTPUT => {
				tracing::info!("Binding output");
				client.insert(new_id.object_id, Output);
			}
			RegistryGlobals::DMABUF => {
				tracing::info!("Binding dmabuf");
				let dmabuf = client.insert(new_id.object_id, Dmabuf::new());
				dmabuf.send_modifiers(client, new_id.object_id).await?;
			}
			id => {
				tracing::error!(id, "Wayland: failed to bind to registry global");
				return Err(Error::MissingObject(unsafe { ObjectId::from_raw(name) }));
			}
		}

		Ok(())
	}
}

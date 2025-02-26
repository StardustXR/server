use crate::wayland::{
	core::{
		compositor::{Compositor, WlCompositor},
		output::{Output, WlOutput},
		seat::{Seat, WlSeat},
		shm::{Shm, WlShm},
	},
	xdg::wm_base::{WmBase, XdgWmBase},
};
pub use waynest::server::protocol::core::wayland::wl_registry::*;
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
				client.insert(new_id.object_id, Compositor);
			}
			RegistryGlobals::SHM => {
				let shm = client.insert(new_id.object_id, Shm);
				shm.advertise_formats(client, new_id.object_id).await?;
			}
			RegistryGlobals::WM_BASE => {
				client.insert(new_id.object_id, WmBase);
			}
			RegistryGlobals::SEAT => {
				client.insert(new_id.object_id, Seat);
			}
			RegistryGlobals::OUTPUT => {
				client.insert(new_id.object_id, Output);
			}
			id => {
				tracing::error!(id, "Wayland: failed to bind to registry global");
				return Err(Error::ObjectNotFound(new_id.object_id));
			}
		}

		Ok(())
	}
}

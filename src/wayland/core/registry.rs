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
	server::{Client, Dispatcher, Error, Object, Result},
	wire::NewId,
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
	pub async fn advertise_globals(&self, object: &Object, client: &mut Client) -> Result<()> {
		self.global(
			object,
			RegistryGlobals::COMPOSITOR,
			Compositor::INTERFACE.to_string(),
			Compositor::VERSION,
		)
		.send(client)
		.await?;

		self.global(
			object,
			RegistryGlobals::SHM,
			Shm::INTERFACE.to_string(),
			Shm::VERSION,
		)
		.send(client)
		.await?;

		self.global(
			object,
			RegistryGlobals::WM_BASE,
			WmBase::INTERFACE.to_string(),
			WmBase::VERSION,
		)
		.send(client)
		.await?;

		self.global(
			object,
			RegistryGlobals::SEAT,
			Seat::INTERFACE.to_string(),
			Seat::VERSION,
		)
		.send(client)
		.await?;

		self.global(
			object,
			RegistryGlobals::OUTPUT,
			Output::INTERFACE.to_string(),
			Output::VERSION,
		)
		.send(client)
		.await?;

		Ok(())
	}
}

impl WlRegistry for Registry {
	async fn bind(
		&self,
		_object: &Object,
		client: &mut Client,
		name: u32,
		new_id: NewId,
	) -> Result<()> {
		match name {
			RegistryGlobals::COMPOSITOR => {
				client.insert(Compositor::default().into_object(new_id.object_id))
			}
			RegistryGlobals::SHM => {
				let shm = Shm::default().into_object(new_id.object_id);

				shm.as_dispatcher::<Shm>()?
					.advertise_formats(&shm, client)
					.await?;

				client.insert(shm);
			}
			RegistryGlobals::WM_BASE => {
				client.insert(WmBase::default().into_object(new_id.object_id))
			}
			RegistryGlobals::SEAT => client.insert(Seat::default().into_object(new_id.object_id)),
			RegistryGlobals::OUTPUT => {
				client.insert(Output::default().into_object(new_id.object_id))
			}
			id => {
				tracing::error!(id, "Wayland: failed to bind to registry global");
				return Err(Error::Internal);
			}
		}

		Ok(())
	}
}

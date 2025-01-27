use crate::wayland::core::{
	callback::{Callback, WlCallback},
	registry::{Registry, WlRegistry},
};
pub use waynest::server::protocol::core::wayland::wl_display::*;
use waynest::{
	server::{Client, Dispatcher, Object, Result},
	wire::ObjectId,
};

#[derive(Debug, Dispatcher, Default)]
pub struct Display;

impl WlDisplay for Display {
	async fn sync(
		&self,
		object: &Object,
		client: &mut Client,
		callback_id: ObjectId,
	) -> Result<()> {
		let serial = client.next_event_serial();

		let callback = Callback::default().into_object(callback_id);

		callback
			.as_dispatcher::<Callback>()?
			.done(&callback, client, serial)
			.await?;

		self.delete_id(object, client, callback_id.as_raw()).await
	}

	async fn get_registry(
		&self,
		_object: &Object,
		client: &mut Client,
		registry_id: ObjectId,
	) -> Result<()> {
		let registry = Registry::default().into_object(registry_id);

		registry
			.as_dispatcher::<Registry>()?
			.advertise_globals(&registry, client)
			.await?;

		client.insert(registry);

		Ok(())
	}
}

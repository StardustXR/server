use crate::wayland::{
	core::{
		callback::{Callback, WlCallback},
		registry::{Registry, WlRegistry},
	},
	MessageSink,
};
pub use waynest::server::protocol::core::wayland::wl_display::*;
use waynest::{
	server::{Client, Dispatcher, Object, Result},
	wire::ObjectId,
};

#[derive(Debug, Dispatcher)]
pub struct Display {
	pub message_sink: MessageSink,
	pub pid: Option<i32>,
}
impl WlDisplay for Display {
	async fn sync(
		&self,
		object: &Object,
		client: &mut Client,
		callback_id: ObjectId,
	) -> Result<()> {
		let serial = client.next_event_serial();

		tracing::info!(serial, "WlDisplay::sync");

		let callback = Callback.into_object(callback_id);

		callback
			.as_dispatcher::<Callback>()?
			.done(&callback, serial)
			.send(client)
			.await?;

		self.delete_id(object, callback_id.as_raw())
			.send(client)
			.await?;
		Ok(())
	}

	async fn get_registry(
		&self,
		_object: &Object,
		client: &mut Client,
		registry_id: ObjectId,
	) -> Result<()> {
		tracing::info!("WlDisplay::get_registry");
		let registry = Registry.into_object(registry_id);

		registry
			.as_dispatcher::<Registry>()?
			.advertise_globals(&registry, client)
			.await?;

		client.insert(registry);

		Ok(())
	}
}

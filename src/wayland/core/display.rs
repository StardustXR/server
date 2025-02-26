use crate::wayland::{
	MessageSink,
	core::{
		callback::{Callback, WlCallback},
		registry::Registry,
	},
};
pub use waynest::server::protocol::core::wayland::wl_display::*;
use waynest::{
	server::{Client, Dispatcher, Result},
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
		client: &mut Client,
		sender_id: ObjectId,
		callback_id: ObjectId,
	) -> Result<()> {
		let serial = client.next_event_serial();
		Callback(callback_id)
			.done(client, callback_id, serial)
			.await?;

		self.delete_id(client, sender_id, callback_id.as_raw())
			.await?;
		Ok(())
	}

	async fn get_registry(
		&self,
		client: &mut Client,
		_sender_id: ObjectId,
		registry_id: ObjectId,
	) -> Result<()> {
		let registry = client.insert(registry_id, Registry);

		registry.advertise_globals(client, registry_id).await?;

		Ok(())
	}
}

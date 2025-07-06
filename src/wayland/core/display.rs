#![allow(unused)]

use crate::wayland::{
	MessageSink,
	core::{
		callback::{Callback, WlCallback},
		registry::Registry,
		seat::Seat,
	},
};
use global_counter::primitive::exact::CounterU32;
use std::{
	sync::{Arc, OnceLock},
	time::Instant,
};
pub use waynest::server::protocol::core::wayland::wl_display::*;
use waynest::{
	server::{Client, Dispatcher, Result},
	wire::ObjectId,
};

#[derive(Dispatcher)]
pub struct Display {
	pub message_sink: MessageSink,
	pub pid: Option<i32>,
	pub seat: OnceLock<Arc<Seat>>,
	id_counter: CounterU32,
	pub creation_time: Instant,
}
impl Display {
	pub fn new(message_sink: MessageSink, pid: Option<i32>) -> Self {
		Self {
			message_sink,
			pid,
			seat: OnceLock::new(),
			id_counter: CounterU32::new(0xff000000), // Start at 0xff000000 to avoid conflicts with client-generated IDs
			creation_time: Instant::now(),
		}
	}
	pub fn next_server_id(&self) -> ObjectId {
		unsafe { ObjectId::from_raw(self.id_counter.inc()) }
	}
}
impl WlDisplay for Display {
	/// https://wayland.app/protocols/wayland#wl_display:request:sync
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

	/// https://wayland.app/protocols/wayland#wl_display:request:get_registry
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

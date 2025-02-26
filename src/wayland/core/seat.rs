pub use waynest::server::protocol::core::wayland::wl_seat::*;
use waynest::{
	server::{Client, Dispatcher, Result},
	wire::ObjectId,
};

#[derive(Debug, Dispatcher, Default)]
pub struct Seat;

impl WlSeat for Seat {
	async fn get_pointer(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_id: ObjectId,
	) -> Result<()> {
		todo!()
	}

	async fn get_keyboard(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_id: ObjectId,
	) -> Result<()> {
		todo!()
	}

	async fn get_touch(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_id: ObjectId,
	) -> Result<()> {
		todo!()
	}

	async fn release(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		todo!()
	}
}

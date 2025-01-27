pub use waynest::server::protocol::core::wayland::wl_seat::*;
use waynest::{
	server::{Client, Dispatcher, Object, Result},
	wire::ObjectId,
};

#[derive(Debug, Dispatcher, Default)]
pub struct Seat;

impl WlSeat for Seat {
	async fn get_pointer(
		&self,
		_object: &Object,
		_client: &mut Client,
		_id: ObjectId,
	) -> Result<()> {
		todo!()
	}

	async fn get_keyboard(
		&self,
		_object: &Object,
		_client: &mut Client,
		_id: ObjectId,
	) -> Result<()> {
		todo!()
	}

	async fn get_touch(&self, _object: &Object, _client: &mut Client, _id: ObjectId) -> Result<()> {
		todo!()
	}

	async fn release(&self, _object: &Object, _client: &mut Client) -> Result<()> {
		todo!()
	}
}

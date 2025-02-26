use waynest::{
	server::{Client, Dispatcher, Result},
	wire::ObjectId,
};

pub use waynest::server::protocol::core::wayland::wl_output::*;

#[derive(Debug, Dispatcher, Default)]
pub struct Output;

impl WlOutput for Output {
	async fn release(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		todo!()
	}
}

use std::os::fd::OwnedFd;

use crate::wayland::core::shm_pool::ShmPool;
pub use waynest::server::protocol::core::wayland::wl_shm::*;
use waynest::{
	server::{Client, Dispatcher, Result},
	wire::ObjectId,
};

#[derive(Debug, Dispatcher, Default)]
pub struct Shm;
impl Shm {
	pub async fn advertise_formats(&self, client: &mut Client, sender_id: ObjectId) -> Result<()> {
		self.format(client, sender_id, Format::Argb8888).await?;
		self.format(client, sender_id, Format::Xrgb8888).await?;

		Ok(())
	}
}
impl WlShm for Shm {
	/// https://wayland.app/protocols/wayland#wl_shm:request:create_pool
	async fn create_pool(
		&self,
		client: &mut Client,
		_sender_id: ObjectId,
		pool_id: ObjectId,
		fd: OwnedFd,
		size: i32,
	) -> Result<()> {
		client.insert(pool_id, ShmPool::new(fd, size)?);

		Ok(())
	}

	/// https://wayland.app/protocols/wayland#wl_shm:request:release
	async fn release(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		Ok(())
	}
}

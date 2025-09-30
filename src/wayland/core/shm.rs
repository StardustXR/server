use crate::wayland::{Client, WaylandResult, core::shm_pool::ShmPool};
use std::os::fd::OwnedFd;
use waynest::ObjectId;
pub use waynest_protocols::server::core::wayland::wl_shm::*;
use waynest_server::Client as _;

#[derive(Debug, waynest_server::RequestDispatcher, Default)]
#[waynest(error = crate::wayland::WaylandError, connection = crate::wayland::Client)]
pub struct Shm;
impl Shm {
	pub async fn advertise_formats(
		&self,
		client: &mut Client,
		sender_id: ObjectId,
	) -> WaylandResult<()> {
		self.format(client, sender_id, Format::Argb8888).await?;
		self.format(client, sender_id, Format::Xrgb8888).await?;

		Ok(())
	}
}
impl WlShm for Shm {
	type Connection = crate::wayland::Client;

	/// https://wayland.app/protocols/wayland#wl_shm:request:create_pool
	async fn create_pool(
		&self,
		client: &mut Self::Connection,
		_sender_id: ObjectId,
		pool_id: ObjectId,
		fd: OwnedFd,
		size: i32,
	) -> WaylandResult<()> {
		client.insert(pool_id, ShmPool::new(fd, size, pool_id)?)?;

		Ok(())
	}

	/// https://wayland.app/protocols/wayland#wl_shm:request:release
	async fn release(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
	) -> WaylandResult<()> {
		Ok(())
	}
}

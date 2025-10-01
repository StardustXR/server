use super::shm_buffer_backing::ShmBufferBacking;
use crate::wayland::{
	Client, WaylandResult,
	core::buffer::{Buffer, BufferBacking},
};
use memmap2::{MmapOptions, RemapOptions};
use parking_lot::{Mutex, MutexGuard, RawMutex, lock_api::MappedMutexGuard};
use std::os::fd::{AsRawFd, OwnedFd};
use waynest::ObjectId;
use waynest_protocols::server::core::wayland::wl_shm::Format;
pub use waynest_protocols::server::core::wayland::wl_shm_pool::*;
use waynest_server::Client as _;

#[derive(Debug, waynest_server::RequestDispatcher)]
#[waynest(error = crate::wayland::WaylandError, connection = crate::wayland::Client)]
pub struct ShmPool {
	inner: Mutex<memmap2::MmapMut>,
	id: ObjectId,
}

impl ShmPool {
	#[tracing::instrument(level = "debug", skip_all)]
	pub fn new(fd: OwnedFd, size: i32, id: ObjectId) -> WaylandResult<Self> {
		let map = unsafe {
			MmapOptions::new()
				.len(size as usize)
				.map_mut(fd.as_raw_fd())?
		};

		Ok(Self {
			inner: Mutex::new(map),
			id,
		})
	}

	#[tracing::instrument(level = "debug", skip_all)]
	pub fn data_lock(&self) -> MappedMutexGuard<'_, RawMutex, [u8]> {
		MutexGuard::map(self.inner.lock(), |i| i.as_mut())
	}
}

impl WlShmPool for ShmPool {
	type Connection = Client;

	/// https://wayland.app/protocols/wayland#wl_shm_pool:request:create_buffer
	#[tracing::instrument(level = "debug", skip_all)]
	async fn create_buffer(
		&self,
		client: &mut Self::Connection,
		sender_id: ObjectId,
		id: ObjectId,
		offset: i32,
		width: i32,
		height: i32,
		stride: i32,
		format: Format,
	) -> WaylandResult<()> {
		let params = ShmBufferBacking::new(
			client.get::<ShmPool>(sender_id).unwrap(),
			offset as usize,
			stride as usize,
			[width as usize, height as usize].into(),
			format,
		);

		Buffer::new(client, id, BufferBacking::Shm(params))?;
		Ok(())
	}

	/// https://wayland.app/protocols/wayland#wl_shm_pool:request:resize
	#[tracing::instrument(level = "debug", skip_all)]
	async fn resize(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		size: i32,
	) -> WaylandResult<()> {
		let mut inner = self.inner.lock();
		unsafe { inner.remap(size as usize, RemapOptions::new().may_move(true))? };
		Ok(())
	}

	/// https://wayland.app/protocols/wayland#wl_shm_pool:request:destroy
	#[tracing::instrument(level = "debug", skip_all)]
	async fn destroy(
		&self,
		client: &mut Self::Connection,
		_sender_id: ObjectId,
	) -> WaylandResult<()> {
		client.remove(self.id);
		Ok(())
	}
}

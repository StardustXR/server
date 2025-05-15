use memmap2::{MmapOptions, RemapOptions};
use parking_lot::{Mutex, MutexGuard, RawMutex, lock_api::MappedMutexGuard};
use std::os::fd::{IntoRawFd, OwnedFd};
use waynest::{
	server::{Client, Dispatcher, Result, protocol::core::wayland::wl_shm::Format},
	wire::ObjectId,
};

use crate::wayland::core::buffer::{Buffer, BufferBacking};

pub use waynest::server::protocol::core::wayland::wl_shm_pool::*;

use super::shm_buffer_backing::ShmBufferBacking;

#[derive(Debug, Dispatcher)]
pub struct ShmPool {
	inner: Mutex<memmap2::MmapMut>,
}

impl ShmPool {
	pub fn new(fd: OwnedFd, size: i32) -> Result<Self> {
		let map = unsafe {
			MmapOptions::new()
				.len(size as usize)
				.map_mut(&fd.into_raw_fd())?
		};

		Ok(Self {
			inner: Mutex::new(map),
		})
	}

	pub fn data_lock(&self) -> MappedMutexGuard<RawMutex, [u8]> {
		MutexGuard::map(self.inner.lock(), |i| i.as_mut())
	}
}

impl WlShmPool for ShmPool {
	async fn create_buffer(
		&self,
		client: &mut Client,
		sender_id: ObjectId,
		id: ObjectId,
		offset: i32,
		width: i32,
		height: i32,
		stride: i32,
		format: Format,
	) -> Result<()> {
		let params = ShmBufferBacking::new(
			client.get::<ShmPool>(sender_id).unwrap(),
			offset as usize,
			stride as usize,
			[width as usize, height as usize].into(),
			format,
		);

		client.insert(
			id,
			Buffer {
				id,
				backing: BufferBacking::Shm(params),
			},
		);
		Ok(())
	}

	async fn resize(&self, _client: &mut Client, _sender_id: ObjectId, size: i32) -> Result<()> {
		let mut inner = self.inner.lock();
		unsafe { inner.remap(size as usize, RemapOptions::new().may_move(true))? };
		Ok(())
	}

	async fn destroy(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		Ok(())
	}
}

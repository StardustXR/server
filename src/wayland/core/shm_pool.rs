use super::buffer::{Buffer, BufferBacking};
use crate::wayland::core::shm::Format;
use memmap2::{MmapMut, MmapOptions, RemapOptions};
use parking_lot::{Mutex, MutexGuard, RawMutex, lock_api::MappedMutexGuard};
use std::os::fd::OwnedFd;

pub use waynest::server::protocol::core::wayland::wl_shm_pool::*;
use waynest::{
	server::{Client, Dispatcher, Result},
	wire::ObjectId,
};

#[derive(Debug)]
struct ShmPoolInner {
	_fd: OwnedFd,
	map: MmapMut,
}

#[derive(Debug, Dispatcher)]
pub struct ShmPool {
	inner: Mutex<ShmPoolInner>,
}

impl ShmPool {
	pub fn new(fd: OwnedFd, size: i32) -> Result<Self> {
		let size = size as usize;
		let file = unsafe { MmapOptions::new().len(size).map_mut(&fd)? };

		Ok(Self {
			inner: Mutex::new(ShmPoolInner { _fd: fd, map: file }),
		})
	}
	pub fn data_lock(&self) -> MappedMutexGuard<RawMutex, [u8]> {
		MutexGuard::map(self.inner.lock(), |i| i.map.as_mut())
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
		client.insert(
			id,
			Buffer::new(
				id,
				offset as usize,
				stride as usize,
				[width as usize, height as usize].into(),
				format,
				BufferBacking::Shm(client.get::<ShmPool>(sender_id).unwrap()),
			),
		);
		Ok(())
	}

	async fn resize(&self, _client: &mut Client, _sender_id: ObjectId, size: i32) -> Result<()> {
		let mut inner = self.inner.lock();
		unsafe {
			inner
				.map
				.remap(size as usize, RemapOptions::new().may_move(true))?
		};
		Ok(())
	}

	async fn destroy(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		Ok(())
	}
}

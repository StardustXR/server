use super::buffer::{Buffer, BufferBacking};
use crate::wayland::core::shm::Format;
use memmap2::{MmapMut, MmapOptions, RemapOptions};
use parking_lot::{lock_api::MappedMutexGuard, Mutex, MutexGuard, RawMutex};
use std::os::fd::OwnedFd;
pub use waynest::server::protocol::core::wayland::wl_shm_pool::*;
use waynest::{
	server::{protocol::core::wayland::wl_buffer::WlBuffer, Client, Dispatcher, Object, Result},
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
		object: &Object,
		client: &mut Client,
		id: ObjectId,
		offset: i32,
		width: i32,
		height: i32,
		stride: i32,
		format: Format,
	) -> Result<()> {
		client.insert(
			Buffer::new(
				id,
				offset as usize,
				stride as usize,
				[width as usize, height as usize].into(),
				format,
				BufferBacking::Shm(object.as_dispatcher()?),
			)
			.into_object(id),
		);
		Ok(())
	}

	async fn resize(&self, _object: &Object, _client: &mut Client, size: i32) -> Result<()> {
		let mut inner = self.inner.lock();
		unsafe {
			inner
				.map
				.remap(size as usize, RemapOptions::new().may_move(true))?
		};
		Ok(())
	}

	async fn destroy(&self, _object: &Object, _client: &mut Client) -> Result<()> {
		Ok(())
	}
}

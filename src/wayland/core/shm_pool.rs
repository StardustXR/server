use super::buffer::Buffer;
use crate::wayland::core::shm::Format;
use memmap2::{MmapMut, MmapOptions, RemapOptions};
use std::os::fd::OwnedFd;
use tokio::sync::{Mutex, RwLock};
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
	inner: Mutex<Option<ShmPoolInner>>,
}

impl ShmPool {
	pub fn new(fd: OwnedFd, size: i32) -> Result<Self> {
		let size = size as usize;
		let file = unsafe { MmapOptions::new().len(size).map_mut(&fd)? };

		Ok(Self {
			inner: Mutex::new(Some(ShmPoolInner { _fd: fd, map: file })),
		})
	}
}

impl WlShmPool for ShmPool {
	async fn create_buffer(
		&self,
		_object: &Object,
		client: &mut Client,
		id: ObjectId,
		offset: i32,
		width: i32,
		height: i32,
		stride: i32,
		format: Format,
	) -> Result<()> {
		client.insert(
			Buffer {
				offset: offset as usize,
				stride: stride as usize,
				size: [width as u32, height as u32].into(),
				format,
			}
			.into_object(id),
		);
		Ok(())
	}

	async fn destroy(&self, _object: &Object, _client: &mut Client) -> Result<()> {
		let mut inner = self.inner.lock().await;
		inner.take();
		Ok(())
	}

	async fn resize(&self, _object: &Object, _client: &mut Client, size: i32) -> Result<()> {
		let mut inner = self.inner.lock().await;
		if let Some(inner) = inner.as_mut() {
			unsafe {
				inner
					.map
					.remap(size as usize, RemapOptions::new().may_move(true))?
			};
		}
		Ok(())
	}
}

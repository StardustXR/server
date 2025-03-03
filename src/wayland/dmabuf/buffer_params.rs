use super::buffer_backing::DmabufBacking;
use crate::wayland::{
	core::buffer::{Buffer, BufferBacking},
	util::ClientExt,
};
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use std::os::fd::OwnedFd;
use waynest::{
	server::{
		Client, Dispatcher, Result,
		protocol::stable::linux_dmabuf_v1::zwp_linux_buffer_params_v1::{
			Flags, ZwpLinuxBufferParamsV1,
		},
	},
	wire::ObjectId,
};

/// A single plane in a DMA-BUF buffer
#[derive(Debug)]
pub struct DmabufPlane {
	pub fd: OwnedFd,
	pub offset: u32,
	pub stride: u32,
	pub _modifier: u64,
}

/// Parameters for creating a DMA-BUF-based wl_buffer
///
/// This is a temporary object that collects dmabufs and other parameters
/// that together form a single logical buffer. The object may eventually
/// create one wl_buffer unless cancelled by destroying it.
#[derive(Debug, Dispatcher)]
pub struct BufferParams {
	pub id: ObjectId,
	planes: Mutex<FxHashMap<u32, DmabufPlane>>,
}

impl BufferParams {
	pub fn new(id: ObjectId) -> Self {
		Self {
			id,
			planes: Mutex::new(FxHashMap::default()),
		}
	}

	pub fn lock_planes(&self) -> parking_lot::MutexGuard<'_, FxHashMap<u32, DmabufPlane>> {
		self.planes.lock()
	}
}

impl Drop for BufferParams {
	fn drop(&mut self) {
		// Clean up any remaining planes - this will close the file descriptors
		self.planes.get_mut().clear();
	}
}

impl ZwpLinuxBufferParamsV1 for BufferParams {
	async fn destroy(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		// Don't clear planes here - they will be cleaned up when the last Arc reference is dropped
		Ok(())
	}

	async fn add(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		fd: OwnedFd,
		plane_idx: u32,
		offset: u32,
		stride: u32,
		modifier_hi: u32,
		modifier_lo: u32,
	) -> Result<()> {
		let mut planes = self.planes.lock();

		// Check if plane index is already set
		if planes.contains_key(&plane_idx) {
			return Err(waynest::server::Error::MissingObject(self.id)); // TODO: Use proper error type when available
		}

		// Create plane with the provided parameters
		let plane = DmabufPlane {
			fd,
			offset,
			stride,
			_modifier: ((modifier_hi as u64) << 32) | (modifier_lo as u64),
		};

		// Store the plane
		planes.insert(plane_idx, plane);
		Ok(())
	}

	async fn create(
		&self,
		client: &mut Client,
		sender_id: ObjectId,
		width: i32,
		height: i32,
		format: u32,
		flags: Flags,
	) -> Result<()> {
		// Create the buffer with DMA-BUF backing using self as the backing
		let size = [width as usize, height as usize].into();
		let backing = DmabufBacking::new(client.get::<Self>(self.id).unwrap(), size, format, flags);
		let id = client.display().next_server_id();
		let buffer = Buffer {
			id,
			backing: BufferBacking::Dmabuf(backing),
		};

		client.insert(id, buffer);

		// Send the created event with the new buffer
		self.created(client, sender_id, id).await
	}

	async fn create_immed(
		&self,
		client: &mut Client,
		_sender_id: ObjectId,
		buffer_id: ObjectId,
		width: i32,
		height: i32,
		format: u32,
		flags: Flags,
	) -> Result<()> {
		// Create the buffer with DMA-BUF backing using self as the backing
		let size = [width as usize, height as usize].into();
		let backing = DmabufBacking::new(client.get::<Self>(self.id).unwrap(), size, format, flags);
		let buffer = Buffer {
			id: buffer_id,
			backing: BufferBacking::Dmabuf(backing),
		};

		client.insert(buffer_id, buffer);
		Ok(())
	}
}

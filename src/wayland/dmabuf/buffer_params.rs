use super::buffer_backing::DmabufBacking;
use crate::wayland::{
	core::buffer::{Buffer, BufferBacking},
	util::ClientExt,
};
use drm_fourcc::DrmFourcc;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use std::os::fd::{AsRawFd, OwnedFd};
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
	pub modifier: u64,
}

/// Parameters for creating a DMA-BUF-based wl_buffer
///
/// This is a temporary object that collects dmabufs and other parameters
/// that together form a single logical buffer. The object may eventually
/// create one wl_buffer unless cancelled by destroying it.
#[derive(Debug, Dispatcher)]
pub struct BufferParams {
	pub id: ObjectId,
	pub(super) planes: Mutex<FxHashMap<u32, DmabufPlane>>,
}

impl BufferParams {
	pub fn new(id: ObjectId) -> Self {
		tracing::info!("Creating new BufferParams with id {:?}", id);
		Self {
			id,
			planes: Mutex::new(FxHashMap::default()),
		}
	}
}

impl ZwpLinuxBufferParamsV1 for BufferParams {
	async fn destroy(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		tracing::info!("Destroying BufferParams {:?}", self.id);
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
		let fd_num = fd.as_raw_fd();
		tracing::info!(
			"Adding plane {} with fd {} to BufferParams {:?}",
			plane_idx,
			fd_num,
			self.id
		);

		let mut planes = self.planes.lock();

		// Check if plane index is already set
		if planes.contains_key(&plane_idx) {
			tracing::error!(
				"Plane {} already exists in BufferParams {:?}",
				plane_idx,
				self.id
			);
			return Err(waynest::server::Error::MissingObject(self.id));
		}

		// Create plane with the provided parameters
		let plane = DmabufPlane {
			fd,
			offset,
			stride,
			modifier: ((modifier_hi as u64) << 32) | (modifier_lo as u64),
		};

		// Store the plane
		planes.insert(plane_idx, plane);
		Ok(())
	}

	async fn create(
		&self,
		client: &mut Client,
		_sender_id: ObjectId,
		width: i32,
		height: i32,
		format: u32,
		flags: Flags,
	) -> Result<()> {
		tracing::info!("Creating buffer from BufferParams {:?}", self.id);
		// Create the buffer with DMA-BUF backing using self as the backing
		let size = [width as u32, height as u32].into();
		let backing = DmabufBacking::new(
			client.get::<Self>(self.id).unwrap(),
			Some(client.display().message_sink.clone()),
			size,
			DrmFourcc::try_from(format).unwrap(),
			flags,
		);
		let id = client.display().next_server_id();
		Buffer::new(client, id, BufferBacking::Dmabuf(backing));

		Ok(())
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
		let backing = DmabufBacking::new(
			client.get::<Self>(self.id).unwrap(),
			None,
			[width as u32, height as u32].into(),
			DrmFourcc::try_from(format).unwrap(),
			flags,
		);
		Buffer::new(client, buffer_id, BufferBacking::Dmabuf(backing));

		Ok(())
	}
}

impl Drop for BufferParams {
	fn drop(&mut self) {
		let planes = self.planes.get_mut();
		tracing::info!("BufferParams being dropped with {} planes", planes.len());
		for (idx, plane) in planes.iter() {
			tracing::info!("Dropping plane {} with fd {}", idx, plane.fd.as_raw_fd());
		}
		planes.clear();
	}
}

use super::buffer_backing::DmabufBacking;
use crate::wayland::{
	Client, WaylandError, WaylandResult,
	core::buffer::{Buffer, BufferBacking},
	util::ClientExt,
};
use bevy_dmabuf::dmatex::DmatexPlane;
use drm_fourcc::DrmFourcc;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use std::os::fd::{AsRawFd, OwnedFd};
use waynest::ObjectId;
use waynest_protocols::server::stable::linux_dmabuf_v1::zwp_linux_buffer_params_v1::{
	Error, Flags, ZwpLinuxBufferParamsV1,
};
use waynest_server::Client as _;

/// Parameters for creating a DMA-BUF-based wl_buffer
///
/// This is a temporary object that collects dmabufs and other parameters
/// that together form a single logical buffer. The object may eventually
/// create one wl_buffer unless cancelled by destroying it.
#[derive(Debug, waynest_server::RequestDispatcher)]
#[waynest(error = crate::wayland::WaylandError, connection = crate::wayland::Client)]
pub struct BufferParams {
	pub id: ObjectId,
	pub(super) planes: Mutex<FxHashMap<u32, DmatexPlane>>,
}

impl BufferParams {
	#[tracing::instrument(level = "debug", skip_all)]
	pub fn new(id: ObjectId) -> Self {
		tracing::info!("Creating new BufferParams with id {:?}", id);
		Self {
			id,
			planes: Mutex::new(FxHashMap::default()),
		}
	}
}

impl ZwpLinuxBufferParamsV1 for BufferParams {
	type Connection = Client;

	async fn destroy(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
	) -> WaylandResult<()> {
		tracing::info!("Destroying BufferParams {:?}", self.id);
		Ok(())
	}

	#[tracing::instrument(level = "debug", skip_all)]
	async fn add(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		fd: OwnedFd,
		plane_idx: u32,
		offset: u32,
		stride: u32,
		modifier_hi: u32,
		modifier_lo: u32,
	) -> WaylandResult<()> {
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
			return Err(crate::wayland::WaylandError::MissingObject(self.id));
		}

		// Create plane with the provided parameters
		let plane = DmatexPlane {
			dmabuf_fd: fd.into(),
			offset,
			stride: stride as i32,
			modifier: ((modifier_hi as u64) << 32) | (modifier_lo as u64),
		};

		// Store the plane
		planes.insert(plane_idx, plane);
		Ok(())
	}

	#[tracing::instrument(level = "debug", skip_all)]
	async fn create(
		&self,
		client: &mut Self::Connection,
		_sender_id: ObjectId,
		width: i32,
		height: i32,
		format: u32,
		flags: Flags,
	) -> WaylandResult<()> {
		tracing::info!("Creating buffer from BufferParams {:?}", self.id);
		// Create the buffer with DMA-BUF backing using self as the backing
		let size = [width as u32, height as u32].into();
		let buffer = DmabufBacking::from_params(
			client.get::<Self>(self.id).unwrap(),
			size,
			DrmFourcc::try_from(format).unwrap(),
			flags,
		)
		.inspect_err(|e| tracing::error!("Failed to import dmabuf because {e}"))
		.map(|backing| {
			let id = client.display().next_server_id();
			Buffer::new(client, id, BufferBacking::Dmabuf(backing))
		});

		match buffer {
			Ok(buffer) => self.created(client, self.id, buffer?.id).await,
			Err(_) => {
				client.remove(self.id);
				self.failed(client, self.id).await
			}
		}
	}

	#[tracing::instrument(level = "debug", skip_all)]
	async fn create_immed(
		&self,
		client: &mut Self::Connection,
		_sender_id: ObjectId,
		buffer_id: ObjectId,
		width: i32,
		height: i32,
		format: u32,
		flags: Flags,
	) -> WaylandResult<()> {
		// TODO: terminate client on fail, or send a fail event or something
		// Create the buffer with DMA-BUF backing using self as the backing
		match DmabufBacking::from_params(
			client.get::<Self>(self.id).unwrap(),
			[width as u32, height as u32].into(),
			DrmFourcc::try_from(format).unwrap(),
			flags,
		) {
			Ok(backing) => {
				Buffer::new(client, buffer_id, BufferBacking::Dmabuf(backing))?;
			}
			Err(e) => {
				tracing::error!("Failed to import dmabuf because {e}");
				return Err(WaylandError::Fatal {
					object_id: buffer_id,
					code: Error::Incomplete as u32,
					message: "Failed to import dmabuf",
				});
			}
		}
		Ok(())
	}
}

impl Drop for BufferParams {
	#[tracing::instrument(level = "debug", skip_all)]
	fn drop(&mut self) {
		let planes = self.planes.get_mut();
		tracing::info!("BufferParams being dropped with {} planes", planes.len());
		for (idx, plane) in planes.iter() {
			tracing::info!(
				"Dropping plane {} with fd {}",
				idx,
				plane.dmabuf_fd.as_raw_fd()
			);
		}
		planes.clear();
	}
}

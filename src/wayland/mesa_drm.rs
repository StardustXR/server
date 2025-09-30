use crate::wayland::{
	Client, WaylandResult,
	core::buffer::{Buffer, BufferBacking},
	dmabuf::{DMABUF_FORMATS, buffer_backing::DmabufBacking},
	vulkano_data::VULKANO_CONTEXT,
};
use bevy_dmabuf::dmatex::{Dmatex, DmatexPlane, Resolution};
use rustc_hash::FxHashSet;
use std::os::fd::OwnedFd;
use waynest::ObjectId;
use waynest_protocols::server::mesa::drm::wl_drm::*;

#[derive(Debug, waynest_server::RequestDispatcher, Default)]
#[waynest(error = crate::wayland::WaylandError, connection = crate::wayland::Client)]
pub struct MesaDrm {
	version: u32,
}
impl MesaDrm {
	pub async fn new(client: &mut Client, id: ObjectId, version: u32) -> WaylandResult<MesaDrm> {
		let drm = MesaDrm { version };

		let path = {
			// Get the device information from Vulkan properties
			let props = VULKANO_CONTEXT.get().unwrap().phys_dev.properties();
			let minor_version = props.render_minor.unwrap();
			format!("/dev/dri/renderD{minor_version}")
		};
		drm.device(client, id, path).await?;

		// this is basically just enabling ancient dmabufs lel
		if drm.version >= 2 {
			drm.capabilities(client, id, Capability::Prime as u32)
				.await?;
		}

		// DRM fomrats check
		let formats = DMABUF_FORMATS
			.iter()
			.map(|(fourcc, _)| fourcc)
			.collect::<FxHashSet<_>>();
		for format in formats {
			drm.format(client, id, *format as u32).await?;
		}

		Ok(drm)
	}
}
impl WlDrm for MesaDrm {
	type Connection = Client;

	async fn authenticate(
		&self,
		client: &mut Self::Connection,
		sender_id: ObjectId,
		_id: u32,
	) -> WaylandResult<()> {
		self.authenticated(client, sender_id).await
	}

	async fn create_buffer(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		_id: ObjectId,
		_name: u32,
		_width: i32,
		_height: i32,
		_stride: u32,
		_format: u32,
	) -> WaylandResult<()> {
		tracing::error!("Tried to create non-prime wl_drm buffer!");
		Ok(())
	}

	async fn create_planar_buffer(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		_id: ObjectId,
		_name: u32,
		_width: i32,
		_height: i32,
		_format: u32,
		_offset0: i32,
		_stride0: i32,
		_offset1: i32,
		_stride1: i32,
		_offset2: i32,
		_stride2: i32,
	) -> WaylandResult<()> {
		tracing::error!("Tried to create non-prime wl_drm buffer!");
		Ok(())
	}

	async fn create_prime_buffer(
		&self,
		client: &mut Self::Connection,
		_sender_id: ObjectId,
		buffer_id: ObjectId,
		name: OwnedFd,
		width: i32,
		height: i32,
		format: u32,
		offset0: i32,
		stride0: i32,
		_offset1: i32,
		_stride1: i32,
		_offset2: i32,
		_stride2: i32,
	) -> WaylandResult<()> {
		// TODO: actual error checking

		let _ = DmabufBacking::new(Dmatex {
			planes: vec![DmatexPlane {
				dmabuf_fd: name.into(),
				modifier: 72057594037927935, // because drmfourcc is so broken it doesn't actually export this, this is Invalid btw
				offset: offset0 as u32,
				stride: stride0,
			}],
			res: Resolution {
				x: width as u32,
				y: height as u32,
			},
			format,
			flip_y: false,
			srgb: true,
		})
		.inspect_err(|e| tracing::error!("Failed to import dmabuf because {e}"))
		.map(|backing| Buffer::new(client, buffer_id, BufferBacking::Dmabuf(backing)));

		Ok(())
	}
}

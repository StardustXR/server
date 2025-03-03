use crate::wayland::{
	GraphicsInfo, core::shm_buffer_backing::ShmBufferBacking, dmabuf::buffer_backing::DmabufBacking,
};
use mint::Vector2;
use stereokit_rust::tex::Tex;
pub use waynest::server::protocol::core::wayland::wl_buffer::*;
use waynest::{
	server::{Client, Dispatcher, Result},
	wire::ObjectId,
};

#[derive(Debug)]
pub enum BufferBacking {
	Shm(ShmBufferBacking),
	Dmabuf(DmabufBacking),
}

#[derive(Debug, Dispatcher)]
pub struct Buffer {
	pub id: ObjectId,
	pub backing: BufferBacking,
}

impl Buffer {
	/// Returns the tex if it was updated
	pub fn update_tex(&self, graphics_info: &GraphicsInfo) -> Option<Tex> {
		match &self.backing {
			BufferBacking::Shm(backing) => backing.update_tex(),
			BufferBacking::Dmabuf(backing) => backing.update_tex(graphics_info),
		}
	}

	pub fn size(&self) -> Vector2<usize> {
		match &self.backing {
			BufferBacking::Shm(backing) => backing.size(),
			BufferBacking::Dmabuf(backing) => backing.size(),
		}
	}
}

impl WlBuffer for Buffer {
	async fn destroy(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		Ok(())
	}
}

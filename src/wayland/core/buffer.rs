use std::sync::Arc;

use crate::{
	core::registry::Registry,
	wayland::{
		GraphicsInfo, core::shm_buffer_backing::ShmBufferBacking,
		dmabuf::buffer_backing::DmabufBacking,
	},
};
use mint::Vector2;
use stereokit_rust::tex::Tex;
pub use waynest::server::protocol::core::wayland::wl_buffer::*;
use waynest::{
	server::{Client, Dispatcher, Result},
	wire::ObjectId,
};

pub static BUFFER_REGISTRY: Registry<Buffer> = Registry::new();

#[derive(Debug)]
pub enum BufferBacking {
	Shm(ShmBufferBacking),
	Dmabuf(DmabufBacking),
}
impl BufferBacking {
	// Returns true if the buffer can be released immediately after texture update
	pub fn can_release_after_update(&self) -> bool {
		matches!(self, BufferBacking::Shm(_))
	}
}

#[derive(Debug, Dispatcher)]
pub struct Buffer {
	pub id: ObjectId,
	backing: BufferBacking,
}

impl Buffer {
	pub fn new(client: &mut Client, id: ObjectId, backing: BufferBacking) -> Arc<Self> {
		let buffer = client.insert(id, Self { id, backing });
		BUFFER_REGISTRY.add_raw(&buffer);
		buffer
	}

	pub fn init_tex(self: Arc<Self>, graphics_info: &Arc<GraphicsInfo>) {
		match &self.backing {
			BufferBacking::Shm(_) => (),
			BufferBacking::Dmabuf(backing) => backing.init_tex(graphics_info, self.clone()),
		}
	}

	/// Returns the tex if it was updated
	pub fn update_tex(&self) -> Option<Tex> {
		tracing::info!("Updating texture for buffer {:?}", self.id);
		match &self.backing {
			BufferBacking::Shm(backing) => backing.update_tex(),
			BufferBacking::Dmabuf(backing) => backing
				.get_tex()
				.map(|tex| tex.get_id().to_string())
				.and_then(|tex_id| Tex::find(tex_id).ok()),
		}
	}

	pub fn can_release_after_update(&self) -> bool {
		self.backing.can_release_after_update()
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
		tracing::info!("Destroying buffer {:?}", self.id);
		Ok(())
	}
}

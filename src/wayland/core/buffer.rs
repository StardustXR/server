use crate::wayland::Message;
use crate::wayland::dmabuf::buffer_backing::DmabufBacking;
use crate::wayland::{MessageSink, core::shm_buffer_backing::ShmBufferBacking, util::ClientExt};
use bevy::{
	asset::{Assets, Handle},
	image::Image,
};
use bevy_dmabuf::import::ImportedDmatexs;
use mint::Vector2;
use std::sync::Arc;
pub use waynest::server::protocol::core::wayland::wl_buffer::*;
use waynest::{
	server::{Client, Dispatcher, Result},
	wire::ObjectId,
};

#[derive(Debug)]
pub struct BufferUsage {
	pub buffer: Arc<Buffer>,
	message_sink: MessageSink,
}
impl BufferUsage {
	pub fn new(client: &Client, buffer: &Arc<Buffer>) -> Arc<Self> {
		Arc::new(Self {
			buffer: buffer.clone(),
			message_sink: client.message_sink(),
		})
	}
}
impl Drop for BufferUsage {
	fn drop(&mut self) {
		let _ = self
			.message_sink
			.send(Message::ReleaseBuffer(self.buffer.clone()));
	}
}

#[derive(Debug)]
pub enum BufferBacking {
	Shm(ShmBufferBacking),
	Dmabuf(DmabufBacking),
}

#[derive(Debug, Dispatcher)]
pub struct Buffer {
	pub id: ObjectId,
	backing: BufferBacking,
}

impl Buffer {
	#[tracing::instrument(level = "debug", skip_all)]
	pub fn new(client: &mut Client, id: ObjectId, backing: BufferBacking) -> Arc<Self> {
		client.insert(id, Self { id, backing })
	}

	/// Returns the tex if it was updated
	#[tracing::instrument(level = "debug", skip_all)]
	pub fn update_tex(
		&self,
		dmatexes: &ImportedDmatexs,
		images: &mut Assets<Image>,
	) -> Option<Handle<Image>> {
		tracing::debug!("Updating texture for buffer {:?}", self.id);
		match &self.backing {
			BufferBacking::Shm(backing) => backing.update_tex(images),
			BufferBacking::Dmabuf(backing) => backing.update_tex(dmatexes, images),
		}
	}

	pub fn is_transparent(&self) -> bool {
		match &self.backing {
			BufferBacking::Shm(backing) => backing.is_transparent(),
			BufferBacking::Dmabuf(backing) => backing.is_transparent(),
		}
	}

	pub fn size(&self) -> Vector2<usize> {
		match &self.backing {
			BufferBacking::Shm(backing) => backing.size(),
			BufferBacking::Dmabuf(backing) => backing.size(),
		}
	}
	pub fn uses_buffer_usage(&self) -> bool {
		matches!(self.backing, BufferBacking::Dmabuf(_))
	}
}

impl WlBuffer for Buffer {
	/// https://wayland.app/protocols/wayland#wl_buffer:request:destroy
	async fn destroy(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		tracing::info!("Destroying buffer {:?}", self.id);
		Ok(())
	}
}

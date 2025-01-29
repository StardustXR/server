use super::shm_pool::ShmPool;
use mint::Vector2;
use nanoid::nanoid;
use parking_lot::Mutex;
use std::sync::Arc;
use stereokit_rust::{
	tex::{Tex, TexFormat, TexType},
	util::Color32,
};
pub use waynest::server::protocol::core::wayland::wl_buffer::*;
use waynest::{
	server::{protocol::core::wayland::wl_shm::Format, Client, Dispatcher, Object, Result},
	wire::ObjectId,
};

#[derive(Debug, Clone)]
pub enum BufferBacking {
	Shm(Arc<ShmPool>),
	Dmabuf(()),
}
#[derive(Debug, Dispatcher)]
pub struct Buffer {
	pub id: ObjectId,
	offset: usize,
	stride: usize,
	pub size: Vector2<usize>,
	format: Format,
	backing: BufferBacking,
	tex: Mutex<Tex>,
}
impl Buffer {
	pub fn new(
		id: ObjectId,
		offset: usize,
		stride: usize,
		size: Vector2<usize>,
		format: Format,
		backing: BufferBacking,
	) -> Self {
		let tex = Tex::new(
			TexType::ImageNomips | TexType::Dynamic,
			TexFormat::RGBA32,
			nanoid!(),
		);

		Self {
			id,
			offset,
			stride,
			size,
			format,
			backing,
			tex: Mutex::new(tex),
		}
	}
	pub fn update_tex(&self) {
		match &self.backing {
			BufferBacking::Shm(shm_pool) => {
				let pixel_count = self.size.x * self.size.y;
				let mut data = Vec::with_capacity(pixel_count);
				let map_lock = shm_pool.data_lock();
				let mut cursor = self.offset;
				for _ in 0..self.size.x {
					for _ in 0..self.size.x {
						data.push(Color32 {
							a: match self.format {
								Format::Argb8888 => map_lock[cursor],
								Format::Xrgb8888 => 255,
								_ => panic!("what the hell bruh"),
							},
							r: map_lock[cursor + 1],
							g: map_lock[cursor + 2],
							b: map_lock[cursor + 3],
						});

						cursor += 4;
					}
					cursor += self.stride;
				}
				self.tex
					.lock()
					.set_colors32(self.size.x, self.size.y, data.as_slice());
			}
			BufferBacking::Dmabuf(_) => {}
		}
	}
}
impl WlBuffer for Buffer {
	async fn destroy(&self, _object: &Object, _client: &mut Client) -> Result<()> {
		Ok(())
	}
}

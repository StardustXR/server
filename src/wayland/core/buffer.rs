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
	server::{Dispatcher, Result, protocol::core::wayland::wl_shm::Format},
	wire::ObjectId,
};

#[derive(Debug, Clone)]
#[allow(dead_code)]
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
	pub fn update_tex(&self) -> Option<Tex> {
		match &self.backing {
			BufferBacking::Shm(shm_pool) => {
				let pixel_count = self.size.x * self.size.y;
				let mut data = Vec::with_capacity(pixel_count);
				let map_lock = shm_pool.data_lock();
				let mut cursor = self.offset;

				// Calculate maximum cursor position needed - stride is already in bytes
				let max_cursor = self.offset + (self.size.y * self.stride);

				// Check if we have enough data
				if max_cursor > map_lock.len() {
					return None;
				}

				for _y in 0..self.size.y {
					for _x in 0..self.size.x {
						let color = Color32 {
							r: map_lock[cursor + 2], // Red is byte 2
							g: map_lock[cursor + 1], // Green is byte 1
							b: map_lock[cursor + 0], // Blue is byte 0
							a: match self.format {
								Format::Xrgb8888 => 255, // X means ignore alpha, treat as fully opaque
								Format::Argb8888 => map_lock[cursor + 3], // Use alpha from byte 3 for ARGB
								_ => panic!("Unsupported format {:?}", self.format),
							},
						};

						data.push(color);
						cursor += 4;
					}
					cursor += self.stride - (self.size.x * 4);
				}
				let mut tex = self.tex.lock().clone_ref();
				tex.set_colors32(self.size.x, self.size.y, data.as_slice());
				Some(tex)
			}
			BufferBacking::Dmabuf(_) => None,
		}
	}
}
impl WlBuffer for Buffer {}

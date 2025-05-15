use mint::Vector2;
use parking_lot::Mutex;
use std::{ffi::c_void, sync::Arc};
use stereokit_rust::tex::{Tex, TexFormat, TexType};
use waynest::server::protocol::core::wayland::wl_shm::Format;

use super::shm_pool::ShmPool;

/// Parameters for a shared memory buffer
#[derive(Debug)]
pub struct ShmBufferBacking {
	pool: Arc<ShmPool>,
	offset: usize,
	stride: usize,
	size: Vector2<usize>,
	format: Format,
	tex: Mutex<Tex>,
}

impl ShmBufferBacking {
	pub fn new(
		pool: Arc<ShmPool>,
		offset: usize,
		stride: usize,
		size: Vector2<usize>,
		format: Format,
	) -> Self {
		let tex = Tex::new(
			TexType::ImageNomips | TexType::Dynamic,
			TexFormat::RGBA32,
			nanoid::nanoid!(),
		);

		Self {
			pool,
			offset,
			stride,
			size,
			format,
			tex: Mutex::new(tex),
		}
	}

	pub fn update_tex(&self) -> Option<Tex> {
		let data_lock = self.pool.data_lock();
		let mut cursor = self.offset;
		let mut pixels = Vec::with_capacity(self.size.x * self.size.y);

		// Calculate maximum cursor position needed - stride is already in bytes
		let max_cursor = self.offset + (self.size.y * self.stride);

		// Check if we have enough data
		if max_cursor > data_lock.len() {
			return None;
		}

		for _y in 0..self.size.y {
			for _x in 0..self.size.x {
				let [r, g, b, a] = match self.format {
					Format::Xrgb8888 => [
						data_lock[cursor + 2], // Red is byte 2
						data_lock[cursor + 1], // Green is byte 1
						data_lock[cursor],     // Blue is byte 0
						255,                   // X means ignore alpha, treat as fully opaque
					],
					Format::Argb8888 => [
						data_lock[cursor + 2], // Red is byte 2
						data_lock[cursor + 1], // Green is byte 1
						data_lock[cursor],     // Blue is byte 0
						data_lock[cursor + 3], // Alpha is byte 3
					],
					_ => panic!("Unsupported format {:?}", self.format),
				};

				pixels.push([r, g, b, a]);
				cursor += 4;
			}
			cursor += self.stride - (self.size.x * 4);
		}

		let tex_id = self.tex.lock().get_id().to_string();
		let mut tex = Tex::find(tex_id).unwrap();
		tex.set_colors(self.size.x, self.size.y, pixels.as_mut_ptr() as *mut c_void);
		Some(tex)
	}

	pub fn size(&self) -> Vector2<usize> {
		self.size
	}
}

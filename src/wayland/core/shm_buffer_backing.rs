use super::shm_pool::ShmPool;
use bevy::{
	asset::{Assets, Handle, RenderAssetUsages},
	image::Image,
	render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use mint::Vector2;
use parking_lot::Mutex;
use std::sync::Arc;
use waynest::server::protocol::core::wayland::wl_shm::Format;

/// Parameters for a shared memory buffer
#[derive(Debug)]
pub struct ShmBufferBacking {
	pool: Arc<ShmPool>,
	offset: usize,
	stride: usize,
	size: Vector2<usize>,
	format: Format,
	image: Mutex<Handle<Image>>,
}

impl ShmBufferBacking {
	pub fn new(
		pool: Arc<ShmPool>,
		offset: usize,
		stride: usize,
		size: Vector2<usize>,
		format: Format,
	) -> Self {
		Self {
			pool,
			offset,
			stride,
			size,
			format,
			image: Mutex::new(Handle::default()),
		}
	}

	pub fn update_tex(&self, images: &mut Assets<Image>) -> Option<Handle<Image>> {
		let mut image_handle = self.image.lock();
		images.remove(image_handle.id());
		let mut image = Image::new_fill(
			Extent3d {
				width: self.size.x as u32,
				height: self.size.y as u32,
				depth_or_array_layers: 1,
			},
			TextureDimension::D2,
			&[255, 0, 255, 255],
			TextureFormat::Rgba8UnormSrgb,
			RenderAssetUsages::all(),
		);

		let src_data_lock = self.pool.data_lock();
		let mut src_cursor = self.offset;

		// Calculate maximum cursor position needed - stride is already in bytes
		let max_cursor = self.offset + (self.size.y * self.stride);

		// Check if we have enough data
		if max_cursor > src_data_lock.len() {
			return None;
		}

		let dst_data = image.data.get_or_insert_with(|| {
			let length = self.size.x * self.size.y * 4;
			vec![255; length]
		});
		let mut dst_cursor = 0;

		for _y in 0..self.size.y {
			for _x in 0..self.size.x {
				match self.format {
					Format::Xrgb8888 => {
						dst_data[dst_cursor] = src_data_lock[src_cursor + 2]; // Red is byte 2
						dst_data[dst_cursor + 1] = src_data_lock[src_cursor + 1]; // Green is byte 1
						dst_data[dst_cursor + 2] = src_data_lock[src_cursor]; // Blue is byte 0
						dst_data[dst_cursor + 3] = 255; // X means ignore alpha, treat as fully opaque
					}
					Format::Argb8888 => {
						dst_data[dst_cursor] = src_data_lock[src_cursor + 2]; // Red is byte 2
						dst_data[dst_cursor + 1] = src_data_lock[src_cursor + 1]; // Green is byte 1
						dst_data[dst_cursor + 2] = src_data_lock[src_cursor]; // Blue is byte 0
						dst_data[dst_cursor + 3] = src_data_lock[src_cursor + 3]; // Alpha is byte 3
					}
					_ => panic!("Unsupported format {:?}", self.format),
				}
				src_cursor += 4;
				dst_cursor += 4;
			}
			src_cursor += self.stride - (self.size.x * 4);
		}

		*image_handle = images.add(image);
		Some(image_handle.clone())
	}

	pub fn is_transparent(&self) -> bool {
		match self.format {
			Format::Xrgb8888 => false,
			Format::Argb8888 => true,
			_ => true,
		}
	}

	pub fn size(&self) -> Vector2<usize> {
		self.size
	}
}

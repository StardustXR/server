use super::shm_pool::ShmPool;
use bevy::{
	asset::{Assets, Handle, RenderAssetUsages},
	image::Image,
	render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use mint::Vector2;
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
		}
	}

	#[tracing::instrument("debug", skip_all)]
	pub fn update_tex(&self, images: &mut Assets<Image>) -> Option<Handle<Image>> {
		let src_data_lock = self.pool.data_lock();
		let mut src_cursor = self.offset;

		// Calculate maximum cursor position needed - stride is already in bytes
		let max_cursor = self.offset + (self.size.y * self.stride);

		// Check if we have enough data
		if max_cursor > src_data_lock.len() {
			return None;
		}
		let data_len = self.size.x * self.size.y * 4;
		if src_data_lock.len() < data_len {
			return None;
		}
		let mut dst_cursor = 0;

		let mut dst_data = vec![0u8; data_len];
		for _y in 0..self.size.y {
			for _x in 0..self.size.x {
				match self.format {
					Format::Argb8888 | Format::Xrgb8888 => {
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

		let image = Image::new(
			Extent3d {
				width: self.size().x as u32,
				height: self.size().y as u32,
				depth_or_array_layers: 1,
			},
			TextureDimension::D2,
			dst_data,
			TextureFormat::Rgba8UnormSrgb,
			RenderAssetUsages::RENDER_WORLD,
		);

		Some(images.add(image))
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

use super::shm_pool::ShmPool;
use bevy::{
	asset::{Assets, Handle, RenderAssetUsages},
	image::Image,
	render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use mint::Vector2;
use std::sync::{Arc, OnceLock};
use tracing::debug_span;
use waynest_protocols::server::core::wayland::wl_shm::Format;

/// Parameters for a shared memory buffer
pub struct ShmBufferBacking {
	pool: Arc<ShmPool>,
	offset: usize,
	stride: usize,
	size: Vector2<usize>,
	wl_format: Format,
	tex_handle: OnceLock<Handle<Image>>,
}

impl std::fmt::Debug for ShmBufferBacking {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("ShmBufferBacking")
			.field("pool", &self.pool)
			.field("offset", &self.offset)
			.field("stride", &self.stride)
			.field("size", &self.size)
			.field("wl_format", &self.wl_format)
			.field("tex_handle", &self.tex_handle)
			.finish()
	}
}

impl ShmBufferBacking {
	pub fn new(
		pool: Arc<ShmPool>,
		offset: usize,
		stride: usize,
		size: Vector2<usize>,
		wl_format: Format,
	) -> Self {
		Self {
			pool,
			offset,
			stride,
			size,
			wl_format,
			tex_handle: OnceLock::new(),
		}
	}

	#[tracing::instrument("debug", skip_all)]
	pub fn update_tex(&self, images: &mut Assets<Image>) -> Option<Handle<Image>> {
		let _span = debug_span!("copy shm to image").entered();

		let handle = self.tex_handle.get_or_init(|| {
			let texture_format = match self.wl_format {
				Format::Argb8888 | Format::Xrgb8888 => TextureFormat::Bgra8UnormSrgb,
				_ => unimplemented!(),
			};

			let image = Image::new_uninit(
				Extent3d {
					width: self.size.x as u32,
					height: self.size.y as u32,
					depth_or_array_layers: 1,
				},
				TextureDimension::D2,
				texture_format,
				RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
			);

			images.add(image)
		});

		let image = images.get_mut(handle)?;
		let data = image.data.get_or_insert_default();

		// Prepare CPU data - copy line by line to handle stride
		let data_len = self.size.x * self.size.y * 4;
		data.resize(data_len, 0);
		{
			let shm_data = self.pool.data_lock();
			for y in 0..self.size.y {
				let shm_offset = self.offset + (y * self.stride);
				let gpu_offset = y * self.size.x * 4;
				let line_len = self.size.x * 4;

				data[gpu_offset..(gpu_offset + line_len)]
					.copy_from_slice(&shm_data[shm_offset..(shm_offset + line_len)]);
			}
		}

		Some(handle.clone())
	}

	pub fn is_transparent(&self) -> bool {
		match self.wl_format {
			Format::Xrgb8888 => false,
			Format::Argb8888 => true,
			_ => true,
		}
	}

	pub fn size(&self) -> Vector2<usize> {
		self.size
	}
}

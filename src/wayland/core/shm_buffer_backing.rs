use crate::wayland::{RENDER_DEVICE, vulkano_data::VULKANO_CONTEXT};

use super::shm_pool::ShmPool;
use bevy::{
	asset::{Assets, Handle},
	image::Image as BevyImage,
};
use bevy_dmabuf::{
	dmatex::{Dmatex, DmatexPlane, Resolution},
	format_mapping::vk_format_to_drm_fourcc,
	import::{DropCallback, ImportedDmatexs, ImportedTexture, import_texture},
};
use mint::Vector2;
use parking_lot::Mutex;
use std::{
	os::fd::OwnedFd,
	sync::{Arc, OnceLock},
};
use tracing::debug_span;
use vulkano::{
	buffer::{BufferCreateFlags, BufferCreateInfo, BufferUsage},
	command_buffer::{AutoCommandBufferBuilder, CommandBufferUsage, CopyBufferToImageInfo},
	image::{
		Image, ImageAspect, ImageCreateFlags, ImageCreateInfo, ImageLayout, ImageMemory,
		ImageTiling, ImageUsage, sys::RawImage,
	},
	memory::{
		DedicatedAllocation, DeviceMemory, ExternalMemoryHandleType, ExternalMemoryHandleTypes,
		MemoryAllocateInfo, ResourceMemory, allocator::AllocationCreateInfo,
	},
	sync::{self, GpuFuture, Sharing},
};
use waynest::server::protocol::core::wayland::wl_shm::Format;

/// Parameters for a shared memory buffer
pub struct ShmBufferBacking {
	pool: Arc<ShmPool>,
	offset: usize,
	stride: usize,
	size: Vector2<usize>,
	format: Format,
	image: Arc<Image>,
	image_handle: OnceLock<Handle<BevyImage>>,
	pending_imported_dmatex: Mutex<Option<ImportedTexture>>,
}

impl ShmBufferBacking {
	pub fn new(
		pool: Arc<ShmPool>,
		offset: usize,
		stride: usize,
		size: Vector2<usize>,
		format: Format,
	) -> Self {
		// TODO: this might cause a freeze?
		let vk = VULKANO_CONTEXT.wait();
		let bevy_render_dev = RENDER_DEVICE.wait();

		let vk_format = vulkano::format::Format::R8G8B8A8_UNORM;

		let modifiers = vk
			.phys_dev
			.format_properties(vk_format)
			.unwrap()
			.drm_format_modifier_properties
			.into_iter()
			.map(|v| v.drm_format_modifier)
			.collect::<Vec<_>>();

		let raw_image = RawImage::new(
			vk.dev.clone(),
			ImageCreateInfo {
				flags: ImageCreateFlags::empty(),
				image_type: vulkano::image::ImageType::Dim2d,
				format: vk_format,
				view_formats: Vec::new(),
				extent: [size.x as u32, size.y as u32, 1],
				tiling: ImageTiling::DrmFormatModifier,
				usage: ImageUsage::COLOR_ATTACHMENT
					| ImageUsage::SAMPLED
					| ImageUsage::TRANSFER_DST,
				initial_layout: ImageLayout::Undefined,
				drm_format_modifiers: modifiers,
				external_memory_handle_types: ExternalMemoryHandleTypes::DMA_BUF,
				..Default::default()
			},
		)
		.unwrap();
		let (modifier, num_planes) = raw_image.drm_format_modifier().unwrap();
		let mem_reqs = raw_image.memory_requirements()[0];
		let index = vk
			.phys_dev
			.memory_properties()
			.memory_types
			.iter()
			.enumerate()
			.map(|(i, _v)| i as u32)
			.find(|i| mem_reqs.memory_type_bits & (1 << i) != 0)
			.expect("no valid memory type");
		let mem = ResourceMemory::new_dedicated(
			DeviceMemory::allocate(
				vk.dev.clone(),
				MemoryAllocateInfo {
					allocation_size: mem_reqs.layout.size(),
					memory_type_index: index,
					dedicated_allocation: Some(DedicatedAllocation::Image(&raw_image)),
					export_handle_types: ExternalMemoryHandleTypes::DMA_BUF,
					..Default::default()
				},
			)
			.unwrap(),
		);

		let image = Arc::new(match raw_image.bind_memory([mem]) {
			Ok(v) => v,
			Err(_) => panic!("unable to bind memory"),
		});
		let ImageMemory::Normal(mem) = image.memory() else {
			unreachable!()
		};
		let [mem] = mem.as_slice() else {
			unreachable!()
		};
		let fd = OwnedFd::from(
			mem.device_memory()
				.export_fd(ExternalMemoryHandleType::DmaBuf)
				.unwrap(),
		);
		let planes = (0..num_planes)
			.filter_map(|i| {
				Some(match i {
					0 => ImageAspect::MemoryPlane0,
					1 => ImageAspect::MemoryPlane1,
					2 => ImageAspect::MemoryPlane2,
					3 => ImageAspect::MemoryPlane3,
					_ => return None,
				})
			})
			.map(|aspect| {
				let plane_layout = image.subresource_layout(aspect, 0, 0).unwrap();

				DmatexPlane {
					dmabuf_fd: fd.try_clone().unwrap().into(),
					modifier,
					offset: plane_layout.offset as u32,
					stride: plane_layout.row_pitch as i32,
				}
			})
			.collect::<Vec<_>>();
		let dmatex = Dmatex {
			planes,
			res: Resolution {
				x: size.x as u32,
				y: size.y as u32,
			},
			format: vk_format_to_drm_fourcc(vk_format.into()).unwrap() as u32,
			flip_y: false,
			srgb: true,
		};

		let imported_texture = import_texture(bevy_render_dev, dmatex, DropCallback(None)).unwrap();
		Self {
			pool,
			offset,
			stride,
			size,
			format,
			image,
			image_handle: OnceLock::new(),
			pending_imported_dmatex: Mutex::new(Some(imported_texture)),
		}
	}

	#[tracing::instrument("debug", skip_all)]
	pub fn update_tex(
		&self,
		dmatexes: &ImportedDmatexs,
		images: &mut Assets<BevyImage>,
	) -> Option<Handle<BevyImage>> {
		if let Some(tex) = self.pending_imported_dmatex.lock().take() {
			self.image_handle
				.set(dmatexes.insert_imported_dmatex(images, tex))
				.unwrap();
		}
		let vk = VULKANO_CONTEXT.wait();
		let image_handle = self.image_handle.get().unwrap();

		let src_data_lock = self.pool.data_lock();
		let mut src_cursor = self.offset;

		// Calculate maximum cursor position needed - stride is already in bytes
		let max_cursor = self.offset + (self.size.y * self.stride);

		// Check if we have enough data
		if max_cursor > src_data_lock.len() {
			return None;
		}
		let data_len = (self.size.x * self.size.y * 4) as u64;
		let mut dst_cursor = 0;

		let buffer = vulkano::buffer::Buffer::new_slice::<u8>(
			vk.alloc.clone(),
			BufferCreateInfo {
				flags: BufferCreateFlags::empty(),
				sharing: Sharing::Exclusive,
				usage: BufferUsage::TRANSFER_SRC,
				..Default::default()
			},
			AllocationCreateInfo {
				memory_type_filter:
					vulkano::memory::allocator::MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
				..Default::default()
			},
			data_len,
		)
		.unwrap();
		let mut dst_data = buffer.write().unwrap();
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
		drop(dst_data);

		let mut command_buffer = AutoCommandBufferBuilder::primary(
			vk.command_buffer_alloc.clone(),
			vk.queue.queue_family_index(),
			CommandBufferUsage::OneTimeSubmit,
		)
		.unwrap();
		command_buffer
			.copy_buffer_to_image(CopyBufferToImageInfo::buffer_image(
				buffer.clone(),
				self.image.clone(),
			))
			.unwrap();
		let command_buffer = command_buffer.build().unwrap();
		debug_span!("waiting for buffer copy").in_scope(|| {
			sync::now(vk.dev.clone())
				.then_execute(vk.queue.clone(), command_buffer)
				.unwrap()
				.then_signal_fence_and_flush()
				.unwrap()
				.wait(None)
				.unwrap()
		});

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

impl std::fmt::Debug for ShmBufferBacking {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("ShmBufferBacking")
			.field("pool", &self.pool)
			.field("offset", &self.offset)
			.field("stride", &self.stride)
			.field("size", &self.size)
			.field("format", &self.format)
			.field("image", &self.image)
			.field("image_handle", &self.image_handle)
			.finish()
	}
}

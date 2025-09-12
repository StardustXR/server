use super::shm_pool::ShmPool;
use crate::wayland::{RENDER_DEVICE, vulkano_data::VULKANO_CONTEXT};
use bevy::{
	asset::{Assets, Handle},
	image::Image,
};
use bevy_dmabuf::{
	dmatex::{Dmatex, DmatexPlane, Resolution},
	import::{DropCallback, ImportedDmatexs, ImportedTexture, import_texture},
};
use drm_fourcc::DrmFourcc;
use mint::Vector2;
use parking_lot::Mutex;
use std::{
	os::fd::OwnedFd,
	sync::{Arc, OnceLock},
};
use tracing::debug_span;
use vulkano::{
	buffer::BufferUsage,
	command_buffer::{
		AutoCommandBufferBuilder, CommandBufferUsage, CopyBufferToImageInfo,
		PrimaryCommandBufferAbstract,
	},
	image::{
		ImageAspect, ImageCreateFlags, ImageCreateInfo, ImageMemory, ImageTiling, ImageUsage,
		sys::RawImage,
	},
	memory::{
		DedicatedAllocation, DeviceMemory, ExternalMemoryHandleType, MemoryAllocateInfo,
		ResourceMemory,
		allocator::{AllocationCreateInfo, MemoryTypeFilter},
	},
	sync::GpuFuture,
};
use waynest::server::protocol::core::wayland::wl_shm::Format;

/// Parameters for a shared memory buffer
pub struct ShmBufferBacking {
	pool: Arc<ShmPool>,
	offset: usize,
	stride: usize,
	size: Vector2<usize>,
	wl_format: Format,
	image: Arc<vulkano::image::Image>,
	tex: OnceLock<Handle<Image>>,
	pending_imported_dmatex: Mutex<Option<ImportedTexture>>,
}

impl std::fmt::Debug for ShmBufferBacking {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("ShmBufferBacking")
			.field("pool", &self.pool)
			.field("offset", &self.offset)
			.field("stride", &self.stride)
			.field("size", &self.size)
			.field("wl_format", &self.wl_format)
			.field("image", &self.image)
			.field("tex", &self.tex)
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
		let vk = VULKANO_CONTEXT.wait();
		let format = match wl_format {
			Format::Argb8888 | Format::Xrgb8888 => vulkano::format::Format::B8G8R8A8_SRGB,
			_ => unimplemented!(),
		};
		let modifiers = vk
			.phys_dev
			.format_properties(format)
			.unwrap()
			.drm_format_modifier_properties
			.into_iter()
			.filter_map(|v| {
				(v.drm_format_modifier_plane_count == 1).then_some(v.drm_format_modifier)
			})
			.collect();
		let raw_image = RawImage::new(
			vk.dev.clone(),
			ImageCreateInfo {
				flags: ImageCreateFlags::empty(),
				image_type: vulkano::image::ImageType::Dim2d,
				format,
				extent: [size.x as u32, size.y as u32, 1],
				tiling: ImageTiling::DrmFormatModifier,
				usage: ImageUsage::TRANSFER_DST,
				drm_format_modifiers: modifiers,
				external_memory_handle_types: ExternalMemoryHandleType::DmaBuf.into(),
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
					export_handle_types: ExternalMemoryHandleType::DmaBuf.into(),
					..Default::default()
				},
			)
			.unwrap(),
		);
		let Ok(image) = raw_image.bind_memory([mem]) else {
			panic!("unable to bind memory")
		};
		let image = Arc::new(image);
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
			format: DrmFourcc::Argb8888 as u32,
			flip_y: false,
			srgb: true,
		};
		let imported_dmatex =
			import_texture(RENDER_DEVICE.wait(), dmatex, DropCallback(None)).unwrap();
		Self {
			pool,
			offset,
			stride,
			size,
			wl_format,
			image,
			pending_imported_dmatex: Mutex::new(Some(imported_dmatex)),
			tex: OnceLock::new(),
		}
	}
	pub fn on_commit(&self) {
		let vk = VULKANO_CONTEXT.wait();
		let data_len = self.size.x * self.size.y * 4;
		let gpu_buffer = vulkano::buffer::Buffer::new_slice::<u8>(
			vk.alloc.clone(),
			vulkano::buffer::BufferCreateInfo {
				usage: BufferUsage::TRANSFER_SRC,
				..Default::default()
			},
			AllocationCreateInfo {
				memory_type_filter: MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
				..Default::default()
			},
			data_len as u64,
		)
		.unwrap();
		{
			let _span = debug_span!("copy to gpu buffer").entered();
			let shm_data_lock = self.pool.data_lock();
			let mut gpu_slice = gpu_buffer.write().unwrap();
			for (shm_offset, gpu_offset) in
				(0..self.size.y).map(|v| (self.offset + (v * self.stride), (v * (self.size.x * 4))))
			{
				let line_slice = &shm_data_lock[shm_offset..(shm_offset + (self.size.x * 4))];
				let gpu_subslice = &mut gpu_slice[gpu_offset..(gpu_offset + (self.size.x * 4))];
				gpu_subslice.copy_from_slice(line_slice);
			}
		}
		let mut command_buffer = AutoCommandBufferBuilder::primary(
			vk.command_buffer_alloc.clone(),
			vk.queue.queue_family_index(),
			CommandBufferUsage::OneTimeSubmit,
		)
		.unwrap();

		command_buffer
			.copy_buffer_to_image(CopyBufferToImageInfo::buffer_image(
				gpu_buffer.clone(),
				self.image.clone(),
			))
			.unwrap();

		let command_buffer = command_buffer.build().unwrap();
		command_buffer
			.execute(vk.queue.clone())
			.unwrap()
			.then_signal_fence_and_flush()
			.unwrap()
			.wait(None)
			.unwrap();
	}

	#[tracing::instrument("debug", skip_all)]
	pub fn update_tex(
		&self,
		dmatexes: &ImportedDmatexs,
		images: &mut Assets<Image>,
	) -> Option<Handle<Image>> {
		self.pending_imported_dmatex
			.lock()
			.take()
			.map(|tex| dmatexes.insert_imported_dmatex(images, tex))
			.inspect(|handle| {
				_ = self.tex.set(handle.clone());
			});
		self.tex.get().cloned()
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

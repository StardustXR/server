use std::sync::{Arc, OnceLock};

use bevy::{
	ecs::system::Res,
	render::renderer::{RenderAdapter, RenderDevice, RenderInstance},
};
use vulkano::{
	VulkanLibrary,
	command_buffer::allocator::{
		CommandBufferAllocator, StandardCommandBufferAllocator,
		StandardCommandBufferAllocatorCreateInfo,
	},
	device::{DeviceCreateInfo, QueueCreateInfo},
	format::Format,
	instance::InstanceCreateFlags,
	memory::allocator::{MemoryAllocator, StandardMemoryAllocator},
};
use wgpu_hal::vulkan::Api as VulkanHal;

pub static VULKANO_CONTEXT: OnceLock<VulkanoContext> = OnceLock::new();

#[expect(dead_code)]
pub struct VulkanoContext {
	pub instance: Arc<vulkano::instance::Instance>,
	pub phys_dev: Arc<vulkano::device::physical::PhysicalDevice>,
	pub dev: Arc<vulkano::device::Device>,
	pub queue: Arc<vulkano::device::Queue>,
	pub alloc: Arc<dyn MemoryAllocator>,
	pub command_buffer_alloc: Arc<dyn CommandBufferAllocator>,
}

pub fn setup_vulkano_context(
	dev: Res<RenderDevice>,
	instance: Res<RenderInstance>,
	adapter: Res<RenderAdapter>,
) {
	if VULKANO_CONTEXT.get().is_some() {
		return;
	}
	let hal_instance = unsafe { instance.as_hal::<VulkanHal>() }
		.unwrap()
		.shared_instance();

	let ash_instance = hal_instance.raw_instance();
	let vulkan_lib =
		VulkanLibrary::with_loader(AshEntryVulkanoLoader(hal_instance.entry().clone())).unwrap();

	let vulkano_instance = unsafe {
		vulkano::instance::Instance::from_handle(
			vulkan_lib,
			ash_instance.handle(),
			vulkano::instance::InstanceCreateInfo {
				flags: InstanceCreateFlags::empty(),
				max_api_version: Some(vulkano::Version::from(hal_instance.instance_api_version())),
				enabled_extensions: vulkano::instance::InstanceExtensions::from_iter(
					hal_instance
						.extensions()
						.iter()
						.map(|s| s.to_str().unwrap()),
				),
				..Default::default()
			},
		)
	};

	let ash_phys_dev_handle = unsafe {
		adapter.as_hal::<VulkanHal, _, _>(|adapter| adapter.unwrap().raw_physical_device())
	};

	let vulkano_phys_dev = unsafe {
		vulkano::device::physical::PhysicalDevice::from_handle(
			vulkano_instance.clone(),
			ash_phys_dev_handle,
		)
	}
	.unwrap();
	let (ash_dev_handle, dev_create_info) = unsafe {
		dev.wgpu_device().as_hal::<VulkanHal, _, _>(|dev| {
			let dev = dev.unwrap();
			(
				dev.raw_device().handle(),
				DeviceCreateInfo {
					queue_create_infos: vec![QueueCreateInfo {
						queue_family_index: dev.queue_family_index(),
						..Default::default()
					}],
					enabled_extensions: dbg!(vulkano::device::DeviceExtensions::from_iter(
						dev.enabled_device_extensions()
							.iter()
							// TODO: remove this hack by telling wgpu about the actual exts used in
							// bevy_mod_openxr
							.chain(bevy_dmabuf::required_device_extensions().iter())
							.map(|v| v.to_str().unwrap()),
					)),
					// this is def wrong, lets hope it doesn't cause issues....
					enabled_features: vulkano::device::DeviceFeatures::empty(),
					..Default::default()
				},
			)
		})
	};
	let (vulkano_dev, mut queues) = unsafe {
		vulkano::device::Device::from_handle(
			vulkano_phys_dev.clone(),
			ash_dev_handle,
			dev_create_info,
		)
	};

	let alloc = Arc::new(StandardMemoryAllocator::new_default(vulkano_dev.clone()));

	let command_buffer_alloc = Arc::new(StandardCommandBufferAllocator::new(
		vulkano_dev.clone(),
		StandardCommandBufferAllocatorCreateInfo::default(),
	));
	_ = VULKANO_CONTEXT.set(VulkanoContext {
		instance: vulkano_instance,
		phys_dev: vulkano_phys_dev,
		dev: vulkano_dev,
		queue: queues.next().unwrap(),
		alloc,
		command_buffer_alloc,
	});
}

// ensures that we don't destroy the vulkan handles wgpu/bevy use
// TODO: remove once we don't use bevys wgpu instance/device/physical_device
impl Drop for VulkanoContext {
	fn drop(&mut self) {
		panic!("the vulkano context shall never be dropped");
	}
}

struct AshEntryVulkanoLoader(ash::Entry);
unsafe impl vulkano::library::Loader for AshEntryVulkanoLoader {
	unsafe fn get_instance_proc_addr(
		&self,
		instance: ash::vk::Instance,
		name: *const std::os::raw::c_char,
	) -> ash::vk::PFN_vkVoidFunction {
		unsafe { self.0.get_instance_proc_addr(instance, name) }
	}
}

pub const DMA_CAPABLE_FORMATS: [Format; 28] = [
	Format::A2B10G10R10_UNORM_PACK32,
	Format::A2B10G10R10_SINT_PACK32,
	Format::A2R10G10B10_UNORM_PACK32,
	Format::A2R10G10B10_SINT_PACK32,
	Format::B8G8R8_UNORM,
	Format::B8G8R8_SINT,
	Format::R8G8B8A8_UNORM,
	Format::R8G8B8A8_SINT,
	Format::R8G8B8_UNORM,
	Format::R8G8B8_SINT,
	Format::B8G8R8A8_UNORM,
	Format::B8G8R8A8_SINT,
	Format::R16_UNORM,
	Format::R16_SINT,
	Format::R8_UNORM,
	Format::R8_SINT,
	Format::R16G16_UNORM,
	Format::R16G16_SINT,
	Format::R8G8_UNORM,
	Format::R8G8_SINT,
	Format::A4B4G4R4_UNORM_PACK16,
	Format::A1R5G5B5_UNORM_PACK16,
	Format::B5G6R5_UNORM_PACK16,
	Format::B4G4R4A4_UNORM_PACK16,
	Format::B5G5R5A1_UNORM_PACK16,
	Format::R5G6B5_UNORM_PACK16,
	Format::R4G4B4A4_UNORM_PACK16,
	Format::R5G5B5A1_UNORM_PACK16,
];

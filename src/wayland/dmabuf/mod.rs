pub mod buffer_backing;
pub mod buffer_params;
pub mod feedback;

use std::sync::LazyLock;

use super::{util::ClientExt, vulkano_data::VULKANO_CONTEXT};
use crate::core::registry::Registry;
use bevy_dmabuf::{format_mapping::drm_fourcc_to_vk_format, wgpu_init::vulkan_to_wgpu};
use buffer_params::BufferParams;
use drm_fourcc::DrmFourcc;
use feedback::DmabufFeedback;
use rustc_hash::FxHashSet;
use waynest::{
	server::{
		Client, Dispatcher, Error, Result,
		protocol::{
			core::wayland::wl_display::WlDisplay,
			stable::linux_dmabuf_v1::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
		},
	},
	wire::ObjectId,
};

pub static DMABUF_FORMATS: LazyLock<FxHashSet<(DrmFourcc, u64)>> = LazyLock::new(|| {
	let vk = VULKANO_CONTEXT.wait();

	ALL_DRM_FOURCCS
		.iter()
		.copied()
		.filter_map(|f| Some((f, drm_fourcc_to_vk_format(f)?)))
		.filter(|(_, vk_format)| vulkan_to_wgpu(*vk_format).is_some())
		.filter_map(|(f, vk_format)| {
			Some((
				f,
				vk.phys_dev
					.format_properties(vk_format.try_into().unwrap())
					.ok()?
					.drm_format_modifier_properties
					.into_iter()
					.map(|v| v.drm_format_modifier)
					.collect::<Vec<_>>(),
			))
		})
		.flat_map(|(f, mods)| mods.into_iter().map(move |modifier| (f, modifier)))
		.collect()
});

/// Main DMA-BUF interface implementation
///
/// This interface allows clients to create wl_buffers from DMA-BUFs.
/// It handles:
/// - Format/modifier advertisement
/// - Buffer parameter creation
/// - Default/surface-specific feedback
///
/// The implementation ensures:
/// - Coherency for read access in dmabuf data
/// - Proper lifetime management of dmabuf file descriptors
/// - Safe handling of buffer attachments
#[derive(Debug, Dispatcher)]
pub struct Dmabuf {
	// Track supported formats and modifiers
	// formats: Mutex<FxHashSet<DrmFormat>>,
	// Track active buffer parameters objects by their ID
	active_params: Registry<BufferParams>,
	pub(self) version: u32,
	pub(self) formats: FxHashSet<(DrmFourcc, u64)>,
}

impl Dmabuf {
	/// Create a new DMA-BUF interface instance
	pub async fn new(client: &mut Client, id: ObjectId, version: u32) -> Result<Self> {
		let dmabuf = Self {
			active_params: Registry::new(),
			version,
			formats: DMABUF_FORMATS.clone(),
		};

		if version < 3 {
			for (format, _) in &dmabuf.formats {
				dmabuf.format(client, id, *format as u32).await?;
			}
		}
		// `modifier` is deprecated in version 4
		if version == 3 {
			for (format, modifier) in &dmabuf.formats {
				let format = *format as u32;
				let modifier_hi = (*modifier >> 32) as u32;
				let modifier_lo = *modifier as u32;
				dmabuf
					.modifier(client, id, format, modifier_hi, modifier_lo)
					.await?;
			}
		}

		Ok(dmabuf)
	}

	/// Remove a buffer parameters object from tracking
	pub(crate) fn remove_params(&self, params_id: ObjectId) {
		self.active_params.retain(|params| params.id != params_id);
	}
}

impl ZwpLinuxDmabufV1 for Dmabuf {
	async fn destroy(&self, _client: &mut Client, sender_id: ObjectId) -> Result<()> {
		self.remove_params(sender_id);
		Ok(())
	}

	async fn create_params(
		&self,
		client: &mut Client,
		_sender_id: ObjectId,
		params_id: ObjectId,
	) -> Result<()> {
		// Create new buffer parameters object
		let params = client.insert(params_id, BufferParams::new(params_id));
		self.active_params.add_raw(&params);
		Ok(())
	}

	async fn get_default_feedback(
		&self,
		client: &mut Client,
		sender_id: ObjectId,
		id: ObjectId,
	) -> Result<()> {
		if self.version < 3 {
			client
				.display()
				.error(
					client,
					sender_id,
					id,
					71,
					"Can't call get_default_feedback on version < 4 of dmabuf".into(),
				)
				.await?;
			return Err(Error::Custom("Protocol error".into()));
		}
		// Create feedback object for default (non-surface-specific) settings
		let feedback = client.insert(id, DmabufFeedback(client.get::<Dmabuf>(sender_id).unwrap()));
		feedback.send_params(client, id).await?;
		Ok(())
	}

	async fn get_surface_feedback(
		&self,
		client: &mut Client,
		sender_id: ObjectId,
		id: ObjectId,
		_surface: ObjectId,
	) -> Result<()> {
		// Create feedback object for surface-specific settings
		// Note: Surface-specific feedback could be optimized based on the surface's
		// requirements, but for now we use the same feedback as default
		self.get_default_feedback(client, sender_id, id).await
	}
}

pub const ALL_DRM_FOURCCS: [DrmFourcc; 105] = [
	DrmFourcc::Abgr1555,
	DrmFourcc::Abgr16161616f,
	DrmFourcc::Abgr2101010,
	DrmFourcc::Abgr4444,
	DrmFourcc::Abgr8888,
	DrmFourcc::Argb1555,
	DrmFourcc::Argb16161616f,
	DrmFourcc::Argb2101010,
	DrmFourcc::Argb4444,
	DrmFourcc::Argb8888,
	DrmFourcc::Axbxgxrx106106106106,
	DrmFourcc::Ayuv,
	DrmFourcc::Bgr233,
	DrmFourcc::Bgr565,
	DrmFourcc::Bgr565_a8,
	DrmFourcc::Bgr888,
	DrmFourcc::Bgr888_a8,
	DrmFourcc::Bgra1010102,
	DrmFourcc::Bgra4444,
	DrmFourcc::Bgra5551,
	DrmFourcc::Bgra8888,
	DrmFourcc::Bgrx1010102,
	DrmFourcc::Bgrx4444,
	DrmFourcc::Bgrx5551,
	DrmFourcc::Bgrx8888,
	DrmFourcc::Bgrx8888_a8,
	DrmFourcc::Big_endian,
	DrmFourcc::C8,
	DrmFourcc::Gr1616,
	DrmFourcc::Gr88,
	DrmFourcc::Nv12,
	DrmFourcc::Nv15,
	DrmFourcc::Nv16,
	DrmFourcc::Nv21,
	DrmFourcc::Nv24,
	DrmFourcc::Nv42,
	DrmFourcc::Nv61,
	DrmFourcc::P010,
	DrmFourcc::P012,
	DrmFourcc::P016,
	DrmFourcc::P210,
	DrmFourcc::Q401,
	DrmFourcc::Q410,
	DrmFourcc::R16,
	DrmFourcc::R8,
	DrmFourcc::Rg1616,
	DrmFourcc::Rg88,
	DrmFourcc::Rgb332,
	DrmFourcc::Rgb565,
	DrmFourcc::Rgb565_a8,
	DrmFourcc::Rgb888,
	DrmFourcc::Rgb888_a8,
	DrmFourcc::Rgba1010102,
	DrmFourcc::Rgba4444,
	DrmFourcc::Rgba5551,
	DrmFourcc::Rgba8888,
	DrmFourcc::Rgbx1010102,
	DrmFourcc::Rgbx4444,
	DrmFourcc::Rgbx5551,
	DrmFourcc::Rgbx8888,
	DrmFourcc::Rgbx8888_a8,
	DrmFourcc::Uyvy,
	DrmFourcc::Vuy101010,
	DrmFourcc::Vuy888,
	DrmFourcc::Vyuy,
	DrmFourcc::X0l0,
	DrmFourcc::X0l2,
	DrmFourcc::Xbgr1555,
	DrmFourcc::Xbgr16161616f,
	DrmFourcc::Xbgr2101010,
	DrmFourcc::Xbgr4444,
	DrmFourcc::Xbgr8888,
	DrmFourcc::Xbgr8888_a8,
	DrmFourcc::Xrgb1555,
	DrmFourcc::Xrgb16161616f,
	DrmFourcc::Xrgb2101010,
	DrmFourcc::Xrgb4444,
	DrmFourcc::Xrgb8888,
	DrmFourcc::Xrgb8888_a8,
	DrmFourcc::Xvyu12_16161616,
	DrmFourcc::Xvyu16161616,
	DrmFourcc::Xvyu2101010,
	DrmFourcc::Xyuv8888,
	DrmFourcc::Y0l0,
	DrmFourcc::Y0l2,
	DrmFourcc::Y210,
	DrmFourcc::Y212,
	DrmFourcc::Y216,
	DrmFourcc::Y410,
	DrmFourcc::Y412,
	DrmFourcc::Y416,
	DrmFourcc::Yuv410,
	DrmFourcc::Yuv411,
	DrmFourcc::Yuv420,
	DrmFourcc::Yuv420_10bit,
	DrmFourcc::Yuv420_8bit,
	DrmFourcc::Yuv422,
	DrmFourcc::Yuv444,
	DrmFourcc::Yuyv,
	DrmFourcc::Yvu410,
	DrmFourcc::Yvu411,
	DrmFourcc::Yvu420,
	DrmFourcc::Yvu422,
	DrmFourcc::Yvu444,
	DrmFourcc::Yvyu,
];

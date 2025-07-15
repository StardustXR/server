pub mod buffer_backing;
pub mod buffer_params;
pub mod feedback;

use bevy_dmabuf::{
	format_mapping::{drm_fourcc_to_vk_format, vk_format_to_drm_fourcc},
	wgpu_init::vulkan_to_wgpu,
};
use buffer_params::BufferParams;
use drm_fourcc::DrmFourcc;
use feedback::DmabufFeedback;
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

use crate::core::registry::Registry;

use super::{
	util::ClientExt,
	vulkano_data::{DMA_CAPABLE_FORMATS, VULKANO_CONTEXT},
};

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
	pub(self) formats: Vec<(DrmFourcc, u64)>,
}

impl Dmabuf {
	/// Create a new DMA-BUF interface instance
	pub async fn new(client: &mut Client, id: ObjectId, version: u32) -> Result<Self> {
		let vk = VULKANO_CONTEXT.wait();
		let formats = DMA_CAPABLE_FORMATS
			.iter()
			.filter(|f| {
				vk_format_to_drm_fourcc((**f).into())
					.and_then(drm_fourcc_to_vk_format)
					.and_then(vulkan_to_wgpu)
					.is_some()
			})
			.filter_map(|f| {
				Some((
					vk_format_to_drm_fourcc((*f).into())?,
					vk.phys_dev
						.format_properties(*f)
						.ok()?
						.drm_format_modifier_properties
						.into_iter()
						.map(|v| v.drm_format_modifier)
						.collect::<Vec<_>>(),
				))
			})
			.flat_map(|(f, mods)| mods.into_iter().map(move |modifier| (f, modifier)))
			.collect();

		let dmabuf = Self {
			// formats: Mutex::new(formats),
			active_params: Registry::new(),
			version,
			formats,
		};

		if version > 3 {
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

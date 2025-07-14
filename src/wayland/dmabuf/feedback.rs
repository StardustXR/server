use bevy_dmabuf::format_mapping::{drm_fourcc_to_vk_format, vk_format_to_drm_fourcc};
use drm_fourcc::DrmFourcc;
use memfd::MemfdOptions;
use std::{
	io::Write,
	os::fd::{FromRawFd, IntoRawFd, OwnedFd},
};
use waynest::{
	server::{
		Client, Dispatcher, Result,
		protocol::stable::linux_dmabuf_v1::zwp_linux_dmabuf_feedback_v1::{
			TrancheFlags, ZwpLinuxDmabufFeedbackV1,
		},
	},
	wire::ObjectId,
};

use crate::wayland::vulkano_data::{DMA_CAPABLE_FORMATS, VULKANO_CONTEXT};

#[derive(Debug, Dispatcher)]
pub struct DmabufFeedback;
impl DmabufFeedback {
	pub async fn send_params(&self, client: &mut Client, sender_id: ObjectId) -> Result<()> {
		let vk = VULKANO_CONTEXT.wait();
		let formats = DMA_CAPABLE_FORMATS
			.iter()
			.filter(|f| {
				vk_format_to_drm_fourcc((**f).into())
					.and_then(drm_fourcc_to_vk_format)
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
			.collect::<Vec<_>>();

		let num_formats = formats.len();
		// Send format table first
		self.send_format_table(client, sender_id, formats).await?;

		// let graphics_info = &client.display().graphics_info;

		// Get the DRM device file path using the new method
		// let device_file = graphics_info.get_drm_device_file_path()?;

		// let dev_stat = std::fs::metadata(device_file)?;
		// let dev_id = dev_stat.rdev().to_ne_bytes().to_vec();

		// self.main_device(client, sender_id, dev_id.clone()).await?;

		// Send single tranche with same device since we only support the main GPU
		// self.tranche_target_device(client, sender_id, dev_id)
		// .await?;

		// let props = vk.phys_dev.properties();
		// tracing::info!(
		// 	props.primary_major,
		// 	props.primary_minor,
		// 	props.render_major,
		// 	props.render_minor
		// );

		// We only have one format at index 0
		let indices = (0..num_formats).flat_map(|i| i.to_ne_bytes()).collect();
		self.tranche_formats(client, sender_id, indices).await?;

		// No special flags needed for simple EGL texture usage
		self.tranche_flags(client, sender_id, TrancheFlags::empty())
			.await?;

		// Mark tranche complete
		self.tranche_done(client, sender_id).await?;

		// Mark overall feedback complete
		self.done(client, sender_id).await?;
		Ok(())
	}

	pub async fn send_format_table(
		&self,
		client: &mut Client,
		sender_id: ObjectId,
		formats: Vec<(DrmFourcc, u64)>,
	) -> Result<()> {
		// Format + modifier pair (16 bytes):
		// - format: u32
		// - padding: 4 bytes
		// - modifier: u64
		let size = formats.len() as u32 * 16u32;
		// Create a temporary file for the format table
		let mfd = MemfdOptions::default()
			.create("stardustxr-format-table")
			.map_err(|e| waynest::server::Error::Custom(e.to_string()))?;

		mfd.as_file().set_len(size as u64)?;

		for (format, modifier) in formats {
			let format = format as u32;
			// Write the format+modifier pair
			mfd.as_file().write_all(&format.to_ne_bytes())?;
			mfd.as_file().write_all(&0_u32.to_ne_bytes())?;
			mfd.as_file().write_all(&modifier.to_ne_bytes())?;
		}

		self.format_table(
			client,
			sender_id,
			unsafe { OwnedFd::from_raw_fd(mfd.into_raw_fd()) },
			size,
		)
		.await?;
		Ok(())
	}
}

impl ZwpLinuxDmabufFeedbackV1 for DmabufFeedback {
	async fn destroy(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		Ok(())
	}
}

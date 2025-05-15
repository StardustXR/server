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

#[derive(Debug, Dispatcher)]
pub struct DmabufFeedback;
impl DmabufFeedback {
	pub async fn send_params(&self, client: &mut Client, sender_id: ObjectId) -> Result<()> {
		// Send format table first
		self.send_format_table(client, sender_id).await?;

		// let graphics_info = &client.display().graphics_info;

		// Get the DRM device file path using the new method
		// let device_file = graphics_info.get_drm_device_file_path()?;

		// let dev_stat = std::fs::metadata(device_file)?;
		// let dev_id = dev_stat.rdev().to_ne_bytes().to_vec();

		// self.main_device(client, sender_id, dev_id.clone()).await?;

		// Send single tranche with same device since we only support the main GPU
		// self.tranche_target_device(client, sender_id, dev_id)
		// .await?;

		// We only have one format at index 0
		let indices = vec![0u16]
			.into_iter()
			.flat_map(|i| i.to_ne_bytes())
			.collect();
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

	pub async fn send_format_table(&self, client: &mut Client, sender_id: ObjectId) -> Result<()> {
		// Create a temporary file for the format table
		let size = 16u32; // One format+modifier pair
		let mfd = MemfdOptions::default()
			.create("stardustxr-format-table")
			.map_err(|e| waynest::server::Error::Custom(e.to_string()))?;

		mfd.as_file().set_len(size as u64)?;

		// Format + modifier pair (16 bytes):
		// - format: u32
		// - padding: 4 bytes
		// - modifier: u64
		let format = DrmFourcc::Xrgb8888 as u32; // This is what clients typically want
		let modifier: u64 = 0; // Linear modifier

		// Write the format+modifier pair
		mfd.as_file().write_all(&format.to_ne_bytes())?;
		mfd.as_file().write_all(&modifier.to_ne_bytes())?;

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

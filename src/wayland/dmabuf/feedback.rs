use super::Dmabuf;
use crate::wayland::vulkano_data::VULKANO_CONTEXT;
use memfd::MemfdOptions;
use std::{
	io::Write,
	os::fd::{FromRawFd, IntoRawFd, OwnedFd},
	sync::Arc,
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
pub struct DmabufFeedback(pub Arc<Dmabuf>);
impl DmabufFeedback {
	#[tracing::instrument(level = "debug", skip_all)]
	pub async fn send_params(&self, client: &mut Client, sender_id: ObjectId) -> Result<()> {
		let num_formats = self.0.formats.len();
		// Send format table first
		self.send_format_table(client, sender_id).await?;

		// Get the device information from Vulkan properties
		let props = VULKANO_CONTEXT.get().unwrap().phys_dev.properties();

		// Create dev_t from the primary node major/minor numbers
		let primary_dev_id = {
			let major = props.primary_major.unwrap() as u64;
			let minor = props.primary_minor.unwrap() as u64;
			// On Linux, dev_t is created with makedev(major, minor)
			// which is ((major & 0xfffff000) << 32) | ((major & 0xfff) << 8) | (minor & 0xff)
			((major & 0xfffff000) << 32) | ((major & 0xfff) << 8) | (minor & 0xff)
		};
		let dev_id = primary_dev_id.to_ne_bytes().to_vec();

		// Send main device
		self.main_device(client, sender_id, dev_id.clone()).await?;

		// Send tranche with same device since we only support the main GPU
		self.tranche_target_device(client, sender_id, dev_id)
			.await?;

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

	#[tracing::instrument(level = "debug", skip_all)]
	pub async fn send_format_table(&self, client: &mut Client, sender_id: ObjectId) -> Result<()> {
		// Format + modifier pair (16 bytes):
		// - format: u32
		// - padding: 4 bytes
		// - modifier: u64
		let size = self.0.formats.len() as u32 * 16u32;
		// Create a temporary file for the format table
		let mfd = MemfdOptions::default()
			.create("stardustxr-format-table")
			.map_err(|e| waynest::server::Error::Custom(e.to_string()))?;

		mfd.as_file().set_len(size as u64)?;

		for (format, modifier) in self.0.formats.iter() {
			let format = *format as u32;
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

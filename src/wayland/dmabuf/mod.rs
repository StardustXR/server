pub mod buffer_backing;
pub mod buffer_params;
pub mod feedback;

use buffer_params::BufferParams;
use drm_fourcc::DrmFourcc;
use feedback::DmabufFeedback;
use waynest::{
	server::{
		Client, Dispatcher, Result,
		protocol::stable::linux_dmabuf_v1::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
	},
	wire::ObjectId,
};

use crate::core::registry::Registry;

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
#[derive(Dispatcher)]
pub struct Dmabuf {
	// Track supported formats and modifiers
	// formats: Mutex<FxHashSet<DrmFormat>>,
	// Track active buffer parameters objects by their ID
	active_params: Registry<BufferParams>,
}

impl Dmabuf {
	/// Create a new DMA-BUF interface instance
	pub fn new() -> Self {
		// let mut formats = FxHashSet::default();

		Self {
			// formats: Mutex::new(formats),
			active_params: Registry::new(),
		}
	}

	pub async fn send_modifiers(&self, client: &mut Client, sender_id: ObjectId) -> Result<()> {
		let format = DrmFourcc::Xrgb8888 as u32;
		let modifier_hi = 0u32; // Linear modifier high 32 bits
		let modifier_lo = 0u32; // Linear modifier low 32 bits
		self.modifier(client, sender_id, format, modifier_hi, modifier_lo)
			.await?;
		Ok(())
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
		_sender_id: ObjectId,
		id: ObjectId,
	) -> Result<()> {
		// Create feedback object for default (non-surface-specific) settings
		let feedback = client.insert(id, DmabufFeedback);
		feedback.send_params(client, id).await?;
		Ok(())
	}

	async fn get_surface_feedback(
		&self,
		client: &mut Client,
		_sender_id: ObjectId,
		id: ObjectId,
		_surface: ObjectId,
	) -> Result<()> {
		// Create feedback object for surface-specific settings
		// Note: Surface-specific feedback could be optimized based on the surface's
		// requirements, but for now we use the same feedback as default
		let feedback = client.insert(id, DmabufFeedback);
		feedback.send_params(client, id).await?;
		Ok(())
	}
}

pub mod buffer_backing;
pub mod buffer_params;
pub mod feedback;

use buffer_params::BufferParams;
use feedback::DmabufFeedback;
use parking_lot::Mutex;
use rustc_hash::FxHashSet;
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
	formats: Mutex<FxHashSet<u32>>,
	// Track active buffer parameters objects by their ID
	active_params: Registry<BufferParams>,
}

impl Dmabuf {
	/// Create a new DMA-BUF interface instance
	pub fn new() -> Self {
		Self {
			formats: Mutex::new(FxHashSet::default()),
			active_params: Registry::new(),
		}
	}

	/// Add a supported format and its modifiers
	pub fn add_format(&self, format: u32) {
		self.formats.lock().insert(format);
	}

	/// Remove a buffer parameters object from tracking
	pub(crate) fn remove_params(&self, params_id: ObjectId) {
		self.active_params.retain(|params| params.id != params_id);
	}
}

impl ZwpLinuxDmabufV1 for Dmabuf {
	async fn destroy(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		// Clean up any resources associated with this instance
		self.active_params.clear();
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
		client.insert(id, DmabufFeedback);
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
		client.insert(id, DmabufFeedback);
		Ok(())
	}
}

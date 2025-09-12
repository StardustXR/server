use rustix::fs::Timespec;
use waynest::{
	server::{
		Client, Dispatcher, Result,
		protocol::stable::presentation_time::{
			wp_presentation::WpPresentation, wp_presentation_feedback::WpPresentationFeedback,
		},
	},
	wire::ObjectId,
};

use crate::wayland::core::surface::Surface;

#[derive(Clone, Copy, Debug)]
pub struct MonotonicTimestamp {
	secs: u64,
	subsec_nanos: u32,
}

impl MonotonicTimestamp {
	pub fn secs_lo(&self) -> u32 {
		self.secs as u32
	}
	pub fn secs_hi(&self) -> u32 {
		(self.secs >> 16) as u32
	}
	pub fn subsec_nanos(&self) -> u32 {
		self.subsec_nanos
	}
}
impl From<Timespec> for MonotonicTimestamp {
	fn from(value: Timespec) -> Self {
		Self {
			secs: value.tv_sec as u64,
			subsec_nanos: value.tv_nsec as u32,
		}
	}
}

#[derive(Debug, Dispatcher)]
pub struct Presentation;
impl WpPresentation for Presentation {
	async fn destroy(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		Ok(())
	}

	async fn feedback(
		&self,
		client: &mut Client,
		_sender_id: ObjectId,
		surface: ObjectId,
		id: ObjectId,
	) -> Result<()> {
		let Some(surface) = client.get::<Surface>(surface) else {
			tracing::error!("unable to get surface#{surface}");
			return Ok(());
		};
		let feedback = client.insert(id, PresentationFeedback(id));
		surface.add_presentation_feedback(feedback);

		Ok(())
	}
}

#[derive(Debug, Dispatcher)]
pub struct PresentationFeedback(pub ObjectId);
impl WpPresentationFeedback for PresentationFeedback {}

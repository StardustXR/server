use crate::wayland::WaylandResult;
use crate::wayland::core::surface::Surface;
use rustix::fs::Timespec;
use waynest::ObjectId;
use waynest_protocols::server::stable::presentation_time::{
	wp_presentation::*, wp_presentation_feedback::*,
};

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

#[derive(Debug, waynest_server::RequestDispatcher)]
#[waynest(error = crate::wayland::WaylandError)]
pub struct Presentation {
	id: ObjectId,
}
impl Presentation {
	pub fn new(id: ObjectId) -> Self {
		Self { id }
	}
}
impl WpPresentation for Presentation {
	type Connection = crate::wayland::Client;

	async fn destroy(
		&self,
		client: &mut Self::Connection,
		_sender_id: ObjectId,
	) -> WaylandResult<()> {
		client.remove(self.id);
		Ok(())
	}

	async fn feedback(
		&self,
		client: &mut Self::Connection,
		_sender_id: ObjectId,
		surface: ObjectId,
		id: ObjectId,
	) -> WaylandResult<()> {
		let Some(surface) = client.get::<Surface>(surface) else {
			tracing::error!("unable to get surface#{surface}");
			return Ok(());
		};
		let feedback = client.insert(id, PresentationFeedback(id));
		surface.add_presentation_feedback(feedback);

		Ok(())
	}
}

#[derive(Debug, waynest_server::RequestDispatcher)]
#[waynest(error = crate::wayland::WaylandError)]
pub struct PresentationFeedback(pub ObjectId);
impl WpPresentationFeedback for PresentationFeedback {
	type Connection = crate::wayland::Client;
}

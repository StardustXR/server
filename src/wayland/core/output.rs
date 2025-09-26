use crate::wayland::{Client, WaylandResult};
use waynest::ObjectId;
pub use waynest_protocols::server::core::wayland::wl_output::*;

#[derive(Debug, waynest_server::RequestDispatcher)]
#[waynest(error = crate::wayland::WaylandError)]
pub struct Output {
	pub id: ObjectId,
	pub version: u32,
}
impl Output {
	pub async fn advertise_outputs(&self, client: &mut Client) -> WaylandResult<()> {
		self.geometry(
			client,
			self.id,
			2048,
			2048,
			0,
			0,
			Subpixel::None,
			"Stardust Virtual Display".to_string(),
			"Stardust Virtual Display".to_string(),
			Transform::Normal,
		)
		.await?;

		if self.version >= 4 {
			self.name(client, self.id, "Stardust Virtual Display".to_string())
				.await?;
			self.description(
				client,
				self.id,
				"I needed this to account for dumb clients".to_string(),
			)
			.await?;
		}
		self.mode(client, self.id, Mode::Current, 2048, 2048, i32::MAX)
			.await?;

		if self.version >= 2 {
			self.done(client, self.id).await?;
		}
		Ok(())
	}
}
impl WlOutput for Output {
	type Connection = Client;

	/// https://wayland.app/protocols/wayland#wl_output:request:release
	async fn release(&self, _client: &mut Self::Connection, _sender_id: ObjectId) -> WaylandResult<()> {
		Ok(())
	}
}

use waynest::{
	server::{Client, Dispatcher, Result},
	wire::ObjectId,
};

pub use waynest::server::protocol::core::wayland::wl_output::*;

#[derive(Debug, Dispatcher)]
pub struct Output(pub ObjectId);
impl Output {
	pub async fn advertise_outputs(&self, client: &mut Client) -> Result<()> {
		self.geometry(
			client,
			self.0,
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

		self.mode(client, self.0, Mode::Current, 2048, 2048, i32::MAX)
			.await?;

		self.done(client, self.0).await
	}
}
impl WlOutput for Output {
	/// https://wayland.app/protocols/wayland#wl_output:request:release
	async fn release(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		Ok(())
	}
}

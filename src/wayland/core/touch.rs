use crate::wayland::{Client, WaylandResult, core::surface::Surface};
use mint::Vector2;
use std::sync::Arc;
use waynest::ObjectId;
pub use waynest_protocols::server::core::wayland::wl_touch::*;

#[derive(Debug, waynest_server::RequestDispatcher)]
#[waynest(error = crate::wayland::WaylandError, connection = crate::wayland::Client)]
pub struct Touch(pub ObjectId);
impl Touch {
	pub async fn handle_touch_down(
		&self,
		client: &mut Client,
		surface: Arc<Surface>,
		id: u32,
		position: Vector2<f32>,
	) -> WaylandResult<()> {
		let serial = client.next_event_serial();
		self.down(
			client,
			self.0,
			serial,
			0,
			surface.id,
			id as i32,
			(position.x as f64).into(),
			(position.y as f64).into(),
		)
		.await?;
		self.frame(client, self.0).await
	}

	pub async fn handle_touch_move(
		&self,
		client: &mut Client,
		id: u32,
		position: Vector2<f32>,
	) -> WaylandResult<()> {
		self.motion(
			client,
			self.0,
			0,
			id as i32,
			(position.x as f64).into(),
			(position.y as f64).into(),
		)
		.await?;
		self.frame(client, self.0).await
	}

	pub async fn handle_touch_up(&self, client: &mut Client, id: u32) -> WaylandResult<()> {
		let serial = client.next_event_serial();
		self.up(client, self.0, serial, 0, id as i32).await?;
		self.frame(client, self.0).await
	}

	pub async fn reset(&self, client: &mut Client) -> WaylandResult<()> {
		self.frame(client, self.0).await
	}
}

impl WlTouch for Touch {
	type Connection = crate::wayland::Client;

	/// https://wayland.app/protocols/wayland#wl_touch:request:release
	async fn release(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
	) -> WaylandResult<()> {
		Ok(())
	}
}

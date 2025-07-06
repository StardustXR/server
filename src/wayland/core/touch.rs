use crate::wayland::core::surface::Surface;
use mint::Vector2;
use std::sync::Arc;
pub use waynest::server::protocol::core::wayland::wl_touch::*;
use waynest::{
	server::{Client, Dispatcher, Result},
	wire::ObjectId,
};

use super::seat::fixed_from_f32;

#[derive(Debug, Dispatcher)]
pub struct Touch(pub ObjectId);
impl Touch {
	pub async fn handle_touch_down(
		&self,
		client: &mut Client,
		surface: Arc<Surface>,
		id: u32,
		position: Vector2<f32>,
	) -> Result<()> {
		let serial = client.next_event_serial();
		self.down(
			client,
			self.0,
			serial,
			0,
			surface.id,
			id as i32,
			fixed_from_f32(position.x),
			fixed_from_f32(position.y),
		)
		.await?;
		self.frame(client, self.0).await
	}

	pub async fn handle_touch_move(
		&self,
		client: &mut Client,
		id: u32,
		position: Vector2<f32>,
	) -> Result<()> {
		self.motion(
			client,
			self.0,
			0,
			id as i32,
			fixed_from_f32(position.x),
			fixed_from_f32(position.y),
		)
		.await?;
		self.frame(client, self.0).await
	}

	pub async fn handle_touch_up(&self, client: &mut Client, id: u32) -> Result<()> {
		let serial = client.next_event_serial();
		self.up(client, self.0, serial, 0, id as i32).await?;
		self.frame(client, self.0).await
	}

	pub async fn reset(&self, client: &mut Client) -> Result<()> {
		self.frame(client, self.0).await
	}
}

impl WlTouch for Touch {
	/// https://wayland.app/protocols/wayland#wl_touch:request:release
	async fn release(
		&self,
		_client: &mut waynest::server::Client,
		_sender_id: waynest::wire::ObjectId,
	) -> Result<()> {
		Ok(())
	}
}

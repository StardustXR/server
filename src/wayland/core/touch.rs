use crate::wayland::core::surface::Surface;
use mint::Vector2;
use std::sync::Arc;
pub use waynest::server::protocol::core::wayland::wl_touch::*;
use waynest::{
	server::{Client, Dispatcher, Result},
	wire::ObjectId,
};

#[derive(Debug, Dispatcher)]
pub struct Touch(pub ObjectId);
impl Touch {
	pub async fn handle_touch_down(
		&self,
		_client: &mut Client,
		_surface: Arc<Surface>,
		_id: u32,
		_position: Vector2<f32>,
	) -> Result<()> {
		// let serial = client.next_event_serial();
		// self.down(
		// 	client,
		// 	self.0,
		// 	serial,
		// 	0,
		// 	surface.id,
		// 	id as i32,
		// 	fixed_from_f32(position.x),
		// 	fixed_from_f32(position.y),
		// )
		// .await
		Ok(())
	}

	pub async fn handle_touch_move(
		&self,
		_client: &mut Client,
		_id: u32,
		_position: Vector2<f32>,
	) -> Result<()> {
		// self.motion(
		// 	client,
		// 	self.0,
		// 	0,
		// 	id as i32,
		// 	fixed_from_f32(position.x),
		// 	fixed_from_f32(position.y),
		// )
		// .await
		Ok(())
	}

	pub async fn handle_touch_up(&self, _client: &mut Client, _id: u32) -> Result<()> {
		// let serial = client.next_event_serial();
		// self.up(client, self.0, serial, 0, id as i32).await
		Ok(())
	}
}

impl WlTouch for Touch {}

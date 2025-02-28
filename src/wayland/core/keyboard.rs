use crate::wayland::core::surface::Surface;
use std::sync::Arc;
pub use waynest::server::protocol::core::wayland::wl_keyboard::*;
use waynest::{
	server::{Client, Dispatcher, Result},
	wire::ObjectId,
};

#[derive(Debug, Dispatcher)]
pub struct Keyboard(pub ObjectId);
impl Keyboard {
	pub async fn handle_keyboard_key(
		&self,
		_client: &mut Client,
		_surface: Arc<Surface>,
		_keymap_id: u64,
		_key: u32,
		_pressed: bool,
	) -> Result<()> {
		// let serial = client.next_event_serial();
		// self.key(
		// 	client,
		// 	self.0,
		// 	serial,
		// 	0,
		// 	key,
		// 	if pressed {
		// 		KeyState::Pressed
		// 	} else {
		// 		KeyState::Released
		// 	},
		// )
		// .await
		Ok(())
	}
}

impl WlKeyboard for Keyboard {}

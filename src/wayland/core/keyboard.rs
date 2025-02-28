use crate::{nodes::items::panel::KEYMAPS, wayland::core::surface::Surface};
use slotmap::{DefaultKey, KeyData};
use std::sync::Arc;
use std::sync::Weak;
use tokio::sync::Mutex;
pub use waynest::server::protocol::core::wayland::wl_keyboard::*;
use waynest::{
	server::{Client, Dispatcher, Result},
	wire::ObjectId,
};

#[derive(Dispatcher)]
pub struct Keyboard {
	pub id: ObjectId,
	focused_surface: Mutex<Weak<Surface>>,
}

impl Keyboard {
	pub fn new(id: ObjectId) -> Self {
		Self {
			id,
			focused_surface: Mutex::new(Weak::new()),
		}
	}

	pub async fn handle_keyboard_key(
		&self,
		client: &mut Client,
		surface: Arc<Surface>,
		keymap_id: u64,
		key: u32,
		pressed: bool,
	) -> Result<()> {
		let mut focused = self.focused_surface.lock().await;

		// If we're entering a new surface
		if focused.as_ptr() != Arc::as_ptr(&surface) {
			// Send leave to old surface if it exists and is still alive
			if let Some(old_surface) = focused.upgrade() {
				let serial = client.next_event_serial();
				self.leave(client, old_surface.id, serial, self.id).await?;
			}

			// Send enter to new surface
			let serial = client.next_event_serial();
			let keymap_key = DefaultKey::from(KeyData::from_ffi(keymap_id));
			let keymap = KEYMAPS
				.lock()
				.get(keymap_key)
				.map(String::as_bytes)
				.unwrap_or_default()
				.to_vec();

			self.enter(client, surface.id, serial, self.id, keymap)
				.await?;

			// Update focused surface
			*focused = Arc::downgrade(&surface);
		}

		// Send key event
		let serial = client.next_event_serial();
		self.key(
			client,
			self.id,
			serial,
			0, // time
			key,
			if pressed {
				KeyState::Pressed
			} else {
				KeyState::Released
			},
		)
		.await?;

		Ok(())
	}
}

impl WlKeyboard for Keyboard {}

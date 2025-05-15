use crate::wayland::core::{keyboard::Keyboard, pointer::Pointer, surface::Surface, touch::Touch};
use mint::Vector2;
use std::sync::Arc;
use std::sync::OnceLock;
pub use waynest::server::protocol::core::wayland::wl_seat::*;
use waynest::wire::Fixed;
use waynest::{
	server::{Client, Dispatcher, Result},
	wire::ObjectId,
};

#[derive(Debug)]
pub enum SeatMessage {
	PointerMotion {
		surface: Arc<Surface>,
		position: Vector2<f32>,
	},
	PointerButton {
		surface: Arc<Surface>,
		button: u32,
		pressed: bool,
	},
	PointerScroll {
		surface: Arc<Surface>,
		scroll_distance: Option<Vector2<f32>>,
		scroll_steps: Option<Vector2<f32>>,
	},
	KeyboardKey {
		surface: Arc<Surface>,
		keymap_id: u64,
		key: u32,
		pressed: bool,
	},
	TouchDown {
		surface: Arc<Surface>,
		id: u32,
		position: Vector2<f32>,
	},
	TouchMove {
		id: u32,
		position: Vector2<f32>,
	},
	TouchUp {
		id: u32,
	},
	Reset,
}

pub fn fixed_from_f32(f: f32) -> Fixed {
	unsafe { Fixed::from_raw((f * 256.0).round() as u32) }
}

#[derive(Default, Dispatcher)]
pub struct Seat {
	pointer: OnceLock<Arc<Pointer>>,
	keyboard: OnceLock<Arc<Keyboard>>,
	touch: OnceLock<Arc<Touch>>,
}

impl Seat {
	pub fn new() -> Self {
		Self::default()
	}

	pub async fn advertise_capabilities(&self, client: &mut Client, id: ObjectId) -> Result<()> {
		tracing::debug!("Advertising seat capabilities with id {}", id);
		let capabilities = Capability::Pointer | Capability::Keyboard | Capability::Touch;
		WlSeat::capabilities(self, client, id, capabilities).await?;
		tracing::debug!("Capabilities advertised: {:?}", capabilities);
		Ok(())
	}

	pub async fn handle_message(&self, client: &mut Client, message: SeatMessage) -> Result<()> {
		match message {
			SeatMessage::PointerMotion { surface, position } => {
				if let Some(pointer) = self.pointer.get() {
					pointer
						.handle_pointer_motion(client, surface, position)
						.await?;
				}
			}
			SeatMessage::PointerButton {
				surface,
				button,
				pressed,
			} => {
				if let Some(pointer) = self.pointer.get() {
					pointer
						.handle_pointer_button(client, surface, button, pressed)
						.await?;
				}
			}
			SeatMessage::PointerScroll {
				surface,
				scroll_distance,
				scroll_steps,
			} => {
				if let Some(pointer) = self.pointer.get() {
					pointer
						.handle_pointer_scroll(client, surface, scroll_distance, scroll_steps)
						.await?;
				}
			}
			SeatMessage::KeyboardKey {
				surface,
				keymap_id,
				key,
				pressed,
			} => {
				tracing::info!("Handling keyboard key message");
				if let Some(keyboard) = self.keyboard.get() {
					tracing::info!("Found keyboard, handling key");
					keyboard
						.handle_keyboard_key(client, surface, keymap_id, key, pressed)
						.await?;
					tracing::info!("Keyboard key handled successfully");
				}
			}
			SeatMessage::TouchDown {
				surface,
				id,
				position,
			} => {
				tracing::info!("Handling touch down message");
				if let Some(touch) = self.touch.get() {
					tracing::info!("Found touch, handling down");
					touch
						.handle_touch_down(client, surface, id, position)
						.await?;
					tracing::info!("Touch down handled successfully");
				}
			}
			SeatMessage::TouchMove { id, position } => {
				tracing::info!("Handling touch move message");
				if let Some(touch) = self.touch.get() {
					tracing::info!("Found touch, handling move");
					touch.handle_touch_move(client, id, position).await?;
					tracing::info!("Touch move handled successfully");
				}
			}
			SeatMessage::TouchUp { id } => {
				tracing::info!("Handling touch up message");
				if let Some(touch) = self.touch.get() {
					tracing::info!("Found touch, handling up");
					touch.handle_touch_up(client, id).await?;
					tracing::info!("Touch up handled successfully");
				}
			}
			SeatMessage::Reset => {
				tracing::info!("Handling reset message");
				if let Some(pointer) = self.pointer.get() {
					pointer.reset(client).await?;
				}
				if let Some(keyboard) = self.keyboard.get() {
					keyboard.reset(client).await?;
				}
				if let Some(touch) = self.touch.get() {
					touch.reset(client).await?;
				}
			}
		}
		Ok(())
	}
}
impl WlSeat for Seat {
	async fn get_pointer(
		&self,
		client: &mut Client,
		_sender_id: ObjectId,
		id: ObjectId,
	) -> Result<()> {
		let pointer = client.insert(id, Pointer::new(id));
		let _ = self.pointer.set(pointer);
		Ok(())
	}

	async fn get_keyboard(
		&self,
		client: &mut Client,
		_sender_id: ObjectId,
		id: ObjectId,
	) -> Result<()> {
		tracing::info!("Getting keyboard");
		let keyboard = client.insert(id, Keyboard::new(id));
		let _ = self.keyboard.set(keyboard);
		Ok(())
	}

	async fn get_touch(
		&self,
		client: &mut Client,
		_sender_id: ObjectId,
		id: ObjectId,
	) -> Result<()> {
		let touch = client.insert(id, Touch(id));
		let _ = self.touch.set(touch);
		Ok(())
	}
}

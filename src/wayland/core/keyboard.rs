use crate::{
	nodes::items::panel::KEYMAPS,
	wayland::{Client, WaylandResult, core::surface::Surface, util::ClientExt},
};
use dashmap::{DashMap, DashSet};
use memfd::MemfdOptions;
use slotmap::{DefaultKey, KeyData};
use std::{
	collections::HashSet,
	io::Write,
	os::{
		fd::IntoRawFd,
		unix::io::{FromRawFd, OwnedFd},
	},
	sync::{Arc, Weak},
};
use tokio::sync::Mutex;
use waynest::ObjectId;
pub use waynest_protocols::server::core::wayland::wl_keyboard::*;

#[derive(Default)]
struct ModifierState {
	pressed_keys: HashSet<u32>,
	mods_depressed: u32,
	mods_latched: u32,
	mods_locked: u32,
	group: u32,
}

impl ModifierState {
	fn update_key(&mut self, key: u32, pressed: bool) -> bool {
		let changed = if pressed {
			self.pressed_keys.insert(key)
		} else {
			self.pressed_keys.remove(&key)
		};

		if changed {
			self.update_modifiers();
		}
		changed
	}

	fn update_modifiers(&mut self) {
		let mut mods = 0;

		// Update modifier state based on currently pressed keys
		for key in &self.pressed_keys {
			match *key {
				input_event_codes::KEY_LEFTSHIFT!() | input_event_codes::KEY_RIGHTSHIFT!() => {
					mods |= 1
				}
				input_event_codes::KEY_LEFTCTRL!() | input_event_codes::KEY_RIGHTCTRL!() => {
					mods |= 4
				}
				input_event_codes::KEY_LEFTALT!() => mods |= 8,
				input_event_codes::KEY_RIGHTALT!() => mods |= 128,
				input_event_codes::KEY_LEFTMETA!() | input_event_codes::KEY_RIGHTMETA!() => {
					mods |= 64
				}
				input_event_codes::KEY_CAPSLOCK!() => self.mods_locked ^= 1,
				_ => {}
			}
		}

		self.mods_depressed = mods;
	}
}

#[derive(waynest_server::RequestDispatcher)]
#[waynest(error = crate::wayland::WaylandError)]
pub struct Keyboard {
	pub id: ObjectId,
	focused_surface: Mutex<Weak<Surface>>,
	modifier_state: Mutex<ModifierState>,
	pressed_keys: DashMap<ObjectId, DashSet<u32>>,
	current_keymap_id: Mutex<u64>,
}

impl Keyboard {
	pub fn new(id: ObjectId) -> Self {
		Self {
			id,
			focused_surface: Mutex::new(Weak::new()),
			modifier_state: Mutex::new(ModifierState::default()),
			pressed_keys: DashMap::default(),
			current_keymap_id: Mutex::new(0),
		}
	}

	async fn send_keymap(&self, client: &mut Client, keymap: &[u8]) -> WaylandResult<()> {
		let mut file = MemfdOptions::default()
			.create("stardust-keymap")?
			.into_file();
		file.set_len(keymap.len() as u64)?;
		file.write_all(keymap)?;
		file.flush()?;

		let fd = unsafe { OwnedFd::from_raw_fd(file.into_raw_fd()) };

		// Send keymap to client
		self.keymap(
			client,
			self.id,
			KeymapFormat::XkbV1,
			fd,
			keymap.len() as u32,
		)
		.await?;

		Ok(())
	}

	/// has to be the wayland key, so -8 or whatever
	pub async fn handle_keyboard_key(
		&self,
		client: &mut Client,
		surface: Arc<Surface>,
		keymap_id: u64,
		key: u32,
		pressed: bool,
	) -> WaylandResult<()> {
		// KEYMAP UPDATES
		{
			let mut old_keymap_id = self.current_keymap_id.lock().await;

			if *old_keymap_id != keymap_id {
				let keymap_key = DefaultKey::from(KeyData::from_ffi(keymap_id));

				// Get keymap data and drop the lock immediately
				let keymap_data = {
					let keymap_lock = KEYMAPS.lock();
					keymap_lock
						.get(keymap_key)
						.map(|s| s.as_bytes().to_vec())
						.unwrap_or_default()
				};

				// Now we can safely await
				self.send_keymap(client, &keymap_data).await?;
			};
			*old_keymap_id = keymap_id;
			drop(old_keymap_id);
		}

		// PRESSED KEYS UPDATE
		let pressed_keys = self.pressed_keys.entry(surface.id).or_default();
		if pressed {
			pressed_keys.insert(key);
		} else {
			pressed_keys.remove(&key);
		}
		// println!("pressed keys: {:?}", &*pressed_keys);

		// FOCUS UPDATES
		let mut focused = self.focused_surface.lock().await;
		let mut modifier_state = self.modifier_state.lock().await;

		let refocus = focused.as_ptr() != Arc::as_ptr(&surface);
		// If we're entering a new surface
		if refocus {
			// Send leave to old surface if it exists and is still alive
			if let Some(old_surface) = focused.upgrade() {
				let serial = client.next_event_serial();
				self.leave(client, old_surface.id, serial, self.id).await?;
				// println!("Left surface {}", old_surface.id);
			}

			// Send enter to new surface
			let serial = client.next_event_serial();
			self.enter(
				client,
				self.id,
				serial,
				surface.id,
				pressed_keys.iter().flat_map(|k| k.to_ne_bytes()).collect(),
			)
			.await?;
			// println!("Entered new surface {}", surface.id);

			// Update focused surface
			*focused = Arc::downgrade(&surface);
		}

		// KEY EVENT SENDING
		let serial = client.next_event_serial();
		// println!(
		// 	"Sent key {key} {}",
		// 	if pressed { "pressed" } else { "released" }
		// );
		self.key(
			client,
			self.id,
			serial,
			client.display().creation_time.elapsed().as_millis() as u32, // time
			key,
			if pressed {
				KeyState::Pressed
			} else {
				KeyState::Released
			},
		)
		.await?;

		// MODIFIER UPDATES
		// Update modifier state and send modifiers event if changed
		if refocus || modifier_state.update_key(key, pressed) {
			// println!("Update modifiers");
			let serial = client.next_event_serial();
			self.modifiers(
				client,
				self.id,
				serial,
				modifier_state.mods_depressed,
				modifier_state.mods_latched,
				modifier_state.mods_locked,
				modifier_state.group,
			)
			.await?;
		}

		Ok(())
	}

	pub async fn reset(&self, client: &mut Client) -> WaylandResult<()> {
		let mut modifier_state = self.modifier_state.lock().await;
		modifier_state.pressed_keys.clear();
		modifier_state.mods_depressed = 0;
		modifier_state.mods_latched = 0;
		modifier_state.mods_locked = 0;
		modifier_state.group = 0;

		let serial = client.next_event_serial();
		self.modifiers(
			client,
			self.id,
			serial,
			modifier_state.mods_depressed,
			modifier_state.mods_latched,
			modifier_state.mods_locked,
			modifier_state.group,
		)
		.await
	}
}

impl WlKeyboard for Keyboard {
	type Connection = Client;

	/// https://wayland.app/protocols/wayland#wl_keyboard:request:release
	async fn release(&self, _client: &mut Self::Connection, _sender_id: ObjectId) -> WaylandResult<()> {
		Ok(())
	}
}

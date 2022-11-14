use super::{state::WaylandState, surface::CoreSurface, GLOBAL_DESTROY_QUEUE};
use crate::nodes::items::Item;
use anyhow::Result;
use mint::Vector2;
use nanoid::nanoid;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use smithay::{
	input::keyboard::{KeymapFile, ModifiersState},
	reexports::wayland_server::{
		backend::{ClientId, GlobalId},
		delegate_dispatch, delegate_global_dispatch,
		protocol::{
			wl_keyboard::{self, KeyState, WlKeyboard},
			wl_pointer::{self, WlPointer},
			wl_seat::{self, Capability, WlSeat, EVT_NAME_SINCE},
			wl_touch::{self, WlTouch},
		},
		Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
	},
	wayland::compositor,
};
use std::sync::Arc;
use std::{ops::Deref, sync::Weak};
use xkbcommon::xkb::{self, Keymap};

pub struct Cursor {
	pub core_surface: Weak<CoreSurface>,
	pub hotspot: Vector2<i32>,
}

pub struct KeyboardInfo {
	pub keymap: KeymapFile,
	pub state: xkb::State,
	pub mods: ModifiersState,
}
impl KeyboardInfo {
	pub fn new(keymap: &Keymap) -> Self {
		KeyboardInfo {
			state: xkb::State::new(keymap),
			keymap: KeymapFile::new(keymap, None),
			mods: ModifiersState::default(),
		}
	}
	pub fn process(&mut self, key: u32, state: u32, keyboard: &WlKeyboard) -> Result<()> {
		let wl_key_state = match state {
			0 => KeyState::Released,
			1 => KeyState::Pressed,
			_ => anyhow::bail!("Invalid key state!"),
		};
		let xkb_key_state = match state {
			0 => xkb::KeyDirection::Up,
			1 => xkb::KeyDirection::Down,
			_ => anyhow::bail!("Invalid key state!"),
		};
		let state_components = self.state.update_key(key + 8, xkb_key_state);
		if state_components != 0 {
			self.mods.update_with(&self.state);
			keyboard.modifiers(
				0,
				self.mods.serialized.depressed,
				self.mods.serialized.latched,
				self.mods.serialized.locked,
				0,
			);
		}
		keyboard.key(0, 0, key, wl_key_state);
		Ok(())
	}
}
unsafe impl Send for KeyboardInfo {}

pub struct SeatDelegate;

#[derive(Clone)]
pub struct SeatData(Arc<SeatDataInner>);
impl SeatData {
	pub fn new(dh: &DisplayHandle, client: ClientId) -> Self {
		let seat_data = SeatData(Arc::new(SeatDataInner {
			client,
			global_id: OnceCell::new(),
			panel_item: OnceCell::new(),
			cursor: Mutex::new(None),
			cursor_changed: Mutex::new(false),
			pointer: OnceCell::new(),
			pointer_active: Mutex::new(false),
			keyboard: OnceCell::new(),
			keyboard_info: Mutex::new(None),
			touch: OnceCell::new(),
		}));

		seat_data
			.global_id
			.set(dh.create_global::<WaylandState, _, _>(7, seat_data.clone()))
			.unwrap();

		seat_data
	}
}
impl Deref for SeatData {
	type Target = SeatDataInner;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}

pub struct SeatDataInner {
	client: ClientId,
	pub global_id: OnceCell<GlobalId>,
	pub panel_item: OnceCell<Weak<Item>>,
	pub cursor: Mutex<Option<Arc<Mutex<Cursor>>>>,
	pub cursor_changed: Mutex<bool>,
	pointer: OnceCell<WlPointer>,
	pub pointer_active: Mutex<bool>,
	keyboard: OnceCell<WlKeyboard>,
	pub keyboard_info: Mutex<Option<KeyboardInfo>>,
	touch: OnceCell<WlTouch>,
}
impl SeatDataInner {
	pub fn pointer(&self) -> Option<&WlPointer> {
		self.pointer.get()
	}
	pub fn pointer_active(&self) -> bool {
		*self.pointer_active.lock()
	}
	pub fn keyboard(&self) -> Option<&WlKeyboard> {
		self.keyboard.get()
	}
	#[allow(dead_code)]
	pub fn touch(&self) -> Option<&WlTouch> {
		self.touch.get()
	}
}
impl Drop for SeatDataInner {
	fn drop(&mut self) {
		let id = self.global_id.take().unwrap();
		tokio::spawn(async move { GLOBAL_DESTROY_QUEUE.get().unwrap().send(id).await });
	}
}

impl GlobalDispatch<WlSeat, SeatData, WaylandState> for SeatDelegate {
	fn bind(
		_state: &mut WaylandState,
		_handle: &DisplayHandle,
		_client: &Client,
		resource: New<WlSeat>,
		data: &SeatData,
		data_init: &mut DataInit<'_, WaylandState>,
	) {
		let resource = data_init.init(resource, data.clone());

		if resource.version() >= EVT_NAME_SINCE {
			resource.name(nanoid!());
		}

		resource.capabilities(Capability::Pointer | Capability::Keyboard);
	}

	fn can_view(client: Client, data: &SeatData) -> bool {
		client.id() == data.0.client
	}
}
delegate_global_dispatch!(WaylandState: [WlSeat: SeatData] => SeatDelegate);

impl Dispatch<WlSeat, SeatData, WaylandState> for SeatDelegate {
	fn request(
		_state: &mut WaylandState,
		_client: &Client,
		_resource: &WlSeat,
		request: <WlSeat as Resource>::Request,
		data: &SeatData,
		_dh: &DisplayHandle,
		data_init: &mut DataInit<'_, WaylandState>,
	) {
		match request {
			wl_seat::Request::GetPointer { id } => {
				let _ = data.0.pointer.set(data_init.init(id, data.clone()));
			}
			wl_seat::Request::GetKeyboard { id } => {
				let keyboard = data_init.init(id, data.clone());
				keyboard.repeat_info(0, 0);
				let _ = data.0.keyboard.set(keyboard);
			}
			wl_seat::Request::GetTouch { id } => {
				let _ = data.0.touch.set(data_init.init(id, data.clone()));
			}
			wl_seat::Request::Release => (),
			_ => unreachable!(),
		}
	}
}
delegate_dispatch!(WaylandState: [WlSeat: SeatData] => SeatDelegate);

impl Dispatch<WlPointer, SeatData, WaylandState> for SeatDelegate {
	fn request(
		state: &mut WaylandState,
		_client: &Client,
		_resource: &WlPointer,
		request: <WlPointer as Resource>::Request,
		seat_data: &SeatData,
		dh: &DisplayHandle,
		_data_init: &mut DataInit<'_, WaylandState>,
	) {
		match request {
			wl_pointer::Request::SetCursor {
				serial: _,
				surface,
				hotspot_x,
				hotspot_y,
			} => {
				// if !seat_data.pointer_active() {
				// 	return;
				// }
				*seat_data.0.cursor_changed.lock() = true;
				if let Some(surface) = surface.as_ref() {
					compositor::with_states(surface, |data| {
						data.data_map.insert_if_missing_threadsafe(|| {
							CoreSurface::new(
								&state.weak_ref.upgrade().unwrap(),
								&state.display,
								dh.clone(),
								surface,
							)
						});
						if !data.data_map.insert_if_missing_threadsafe(|| {
							Arc::new(Mutex::new(Cursor {
								core_surface: Arc::downgrade(
									data.data_map.get::<Arc<CoreSurface>>().unwrap(),
								),
								hotspot: Vector2::from([hotspot_x, hotspot_y]),
							}))
						}) {
							let mut cursor =
								data.data_map.get::<Arc<Mutex<Cursor>>>().unwrap().lock();
							cursor.hotspot = Vector2::from([hotspot_x, hotspot_y]);
						}
					})
				}
				*seat_data.cursor.lock() = surface.and_then(|surf| {
					compositor::with_states(&surf, |data| {
						let cursor = data.data_map.get::<Arc<Mutex<Cursor>>>();
						if let Some(cursor) = cursor {
							if let Some(core_surface) = cursor.lock().core_surface.upgrade() {
								core_surface.set_material_offset(1);
							}
						}
						cursor.cloned()
					})
				});
			}
			wl_pointer::Request::Release => (),
			_ => unreachable!(),
		}
	}
}
delegate_dispatch!(WaylandState: [WlPointer: SeatData] => SeatDelegate);

impl Dispatch<WlKeyboard, SeatData, WaylandState> for SeatDelegate {
	fn request(
		_state: &mut WaylandState,
		_client: &Client,
		_resource: &WlKeyboard,
		request: <WlKeyboard as Resource>::Request,
		_data: &SeatData,
		_dh: &DisplayHandle,
		_data_init: &mut DataInit<'_, WaylandState>,
	) {
		match request {
			wl_keyboard::Request::Release => (),
			_ => unreachable!(),
		}
	}
}
delegate_dispatch!(WaylandState: [WlKeyboard: SeatData] => SeatDelegate);

impl Dispatch<WlTouch, SeatData, WaylandState> for SeatDelegate {
	fn request(
		_state: &mut WaylandState,
		_client: &Client,
		_resource: &WlTouch,
		request: <WlTouch as Resource>::Request,
		_data: &SeatData,
		_dh: &DisplayHandle,
		_data_init: &mut DataInit<'_, WaylandState>,
	) {
		match request {
			wl_touch::Request::Release => (),
			_ => unreachable!(),
		}
	}
}
delegate_dispatch!(WaylandState: [WlTouch: SeatData] => SeatDelegate);

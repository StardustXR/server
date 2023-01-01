use super::{
	panel_item::PanelItem, state::WaylandState, surface::CoreSurface, GLOBAL_DESTROY_QUEUE,
};
use color_eyre::eyre::Result;
use mint::Vector2;
use nanoid::nanoid;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use smithay::{
	input::keyboard::{KeymapFile, ModifiersState},
	reexports::wayland_server::{
		backend::{ClientId, GlobalId},
		protocol::{
			wl_keyboard::{self, KeyState, WlKeyboard},
			wl_pointer::{self, WlPointer},
			wl_seat::{self, Capability, WlSeat, EVT_NAME_SINCE},
			wl_surface::WlSurface,
			wl_touch::{self, WlTouch},
		},
		Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, Weak as WlWeak,
	},
	wayland::compositor,
};
use std::{
	ops::Deref,
	sync::{Arc, Weak},
};
use xkbcommon::xkb::{self, Keymap};

pub struct Cursor {
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
			_ => color_eyre::eyre::bail!("Invalid key state!"),
		};
		let xkb_key_state = match state {
			0 => xkb::KeyDirection::Up,
			1 => xkb::KeyDirection::Down,
			_ => color_eyre::eyre::bail!("Invalid key state!"),
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

#[derive(Clone)]
pub struct SeatData(Arc<SeatDataInner>);
impl SeatData {
	pub fn new(dh: &DisplayHandle, client: ClientId) -> Self {
		let seat_data = SeatData(Arc::new(SeatDataInner {
			client,
			global_id: OnceCell::new(),
			panel_item: OnceCell::new(),
			pointer: OnceCell::new(),
			pointer_focus: Mutex::new(None),
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
	global_id: OnceCell<GlobalId>,
	pub panel_item: OnceCell<Weak<PanelItem>>,
	pointer: OnceCell<WlPointer>,
	pub pointer_focus: Mutex<Option<WlWeak<WlSurface>>>,
	keyboard: OnceCell<WlKeyboard>,
	pub keyboard_info: Mutex<Option<KeyboardInfo>>,
	touch: OnceCell<WlTouch>,
}
impl SeatDataInner {
	pub fn pointer(&self) -> Option<&WlPointer> {
		self.pointer.get()
	}
	pub fn pointer_active(&self) -> bool {
		self.pointer_focus.lock().is_some()
	}
	pub fn pointer_focused_surface(&self) -> Option<WlSurface> {
		self.pointer_focus
			.lock()
			.as_ref()
			.and_then(|focus| focus.upgrade().ok())
	}
	pub fn keyboard(&self) -> Option<&WlKeyboard> {
		self.keyboard.get()
	}
	#[allow(dead_code)]
	pub fn touch(&self) -> Option<&WlTouch> {
		self.touch.get()
	}

	pub fn cursor(&self) -> Option<WlSurface> {
		self.panel_item
			.get()?
			.upgrade()?
			.cursor
			.lock()
			.as_ref()
			.and_then(|c| c.upgrade().ok())
	}
}
impl Drop for SeatDataInner {
	fn drop(&mut self) {
		let id = self.global_id.take().unwrap();
		tokio::spawn(async move { GLOBAL_DESTROY_QUEUE.get().unwrap().send(id).await });
	}
}

impl GlobalDispatch<WlSeat, SeatData, WaylandState> for WaylandState {
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

impl Dispatch<WlSeat, SeatData, WaylandState> for WaylandState {
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

impl Dispatch<WlPointer, SeatData, WaylandState> for WaylandState {
	fn request(
		state: &mut WaylandState,
		_client: &Client,
		_resource: &WlPointer,
		request: wl_pointer::Request,
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
				if let Some(surface) = surface.as_ref() {
					CoreSurface::add_to(&state.display, dh.clone(), surface);
					compositor::with_states(surface, |data| {
						data.data_map.insert_if_missing_threadsafe(|| {
							Arc::new(Mutex::new(Cursor {
								hotspot: Vector2::from([hotspot_x, hotspot_y]),
							}))
						});
						let mut cursor = data.data_map.get::<Arc<Mutex<Cursor>>>().unwrap().lock();
						cursor.hotspot = Vector2::from([hotspot_x, hotspot_y]);

						if let Some(core_surface) = data.data_map.get::<Arc<CoreSurface>>() {
							core_surface.set_material_offset(1);
						}
					})
				}

				if let Some(panel_item) = seat_data.panel_item.get().and_then(|i| i.upgrade()) {
					panel_item.set_cursor(surface.as_ref(), hotspot_x, hotspot_y);
					*panel_item.cursor.lock() = surface.as_ref().map(|surf| surf.downgrade());
				}
			}
			wl_pointer::Request::Release => (),
			_ => unreachable!(),
		}
	}
}

impl Dispatch<WlKeyboard, SeatData, WaylandState> for WaylandState {
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

impl Dispatch<WlTouch, SeatData, WaylandState> for WaylandState {
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

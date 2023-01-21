use crate::core::task;

use super::{
	panel_item::PanelItem, state::WaylandState, surface::CoreSurface, GLOBAL_DESTROY_QUEUE,
	SERIAL_COUNTER,
};
use color_eyre::eyre::Result;
use mint::Vector2;
use nanoid::nanoid;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use rand::{seq::IteratorRandom, thread_rng};
use rustc_hash::{FxHashMap, FxHashSet};
use smithay::{
	input::keyboard::{KeymapFile, ModifiersState},
	reexports::{
		wayland_protocols::xdg::shell::server::xdg_toplevel::XdgToplevel,
		wayland_server::{
			backend::{ClientId, GlobalId, ObjectId},
			protocol::{
				wl_keyboard::{self, KeyState, WlKeyboard},
				wl_pointer::{self, Axis, ButtonState, WlPointer},
				wl_seat::{self, Capability, WlSeat, EVT_NAME_SINCE},
				wl_surface::WlSurface,
				wl_touch::{self, WlTouch},
			},
			Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
			Weak as WlWeak,
		},
	},
	wayland::compositor,
};
use std::{
	collections::VecDeque,
	ops::Deref,
	sync::{Arc, Weak},
	time::{Duration, Instant},
};
use tracing::{debug, warn};
use xkbcommon::xkb::{self, Keymap};

#[derive(Clone)]
pub struct SeatData(Arc<SeatDataInner>);
impl SeatData {
	pub fn new(dh: &DisplayHandle, client: ClientId) -> Self {
		let seat_data = SeatData(Arc::new(SeatDataInner {
			client,
			global_id: OnceCell::new(),
			panels: Mutex::new(FxHashMap::default()),
			pointer: OnceCell::new(),
			keyboard: OnceCell::new(),
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

pub struct KeyboardInfo {
	keymap: KeymapFile,
	state: xkb::State,
	mods: ModifiersState,
	keys: FxHashSet<u32>,
}
impl KeyboardInfo {
	pub fn new(keymap: &Keymap) -> Self {
		KeyboardInfo {
			state: xkb::State::new(keymap),
			keymap: KeymapFile::new(keymap, None),
			mods: ModifiersState::default(),
			keys: FxHashSet::default(),
		}
	}
	pub fn process(&mut self, key: u32, state: u32, keyboard: &WlKeyboard) -> Result<usize> {
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
		keyboard.key(SERIAL_COUNTER.inc(), 0, key, wl_key_state);
		match wl_key_state {
			KeyState::Pressed => {
				self.keys.insert(key);
			}
			KeyState::Released => {
				self.keys.remove(&key);
			}
			_ => unimplemented!(),
		}
		Ok(self.keys.len())
	}
}
unsafe impl Send for KeyboardInfo {}

#[derive(Debug, Clone, Copy)]
pub enum PointerEvent {
	Motion(Vector2<f64>),
	Button {
		button: u32,
		state: u32,
	},
	Scroll {
		axis_continuous: Option<Vector2<f32>>,
		axis_discrete: Option<Vector2<f32>>,
	},
}
#[derive(Debug, Clone)]
pub enum KeyboardEvent {
	Keymap,
	Key { key: u32, state: u32 },
}

const POINTER_EVENT_TIMEOUT: Duration = Duration::from_secs(1);
struct PanelInfo {
	panel_item: Weak<PanelItem>,
	toplevel: WlWeak<XdgToplevel>,
	focus: WlWeak<WlSurface>,
	pointer_queue: Option<VecDeque<PointerEvent>>,
	pointer_latest_event: Instant,
	keyboard_queue: Option<VecDeque<KeyboardEvent>>,
	keyboard_info: Option<KeyboardInfo>,
}
impl PanelInfo {
	fn new(panel_item: &Arc<PanelItem>, toplevel: &XdgToplevel, focus: &WlSurface) -> Self {
		PanelInfo {
			toplevel: toplevel.downgrade(),
			panel_item: Arc::downgrade(panel_item),
			focus: focus.downgrade(),
			pointer_queue: None,
			pointer_latest_event: Instant::now(),
			keyboard_queue: None,
			keyboard_info: None,
		}
	}
	pub fn set_pointer_active(&mut self, seat_data: &SeatDataInner, active: bool) {
		if active && self.pointer_queue.is_none() {
			self.pointer_queue.replace(Default::default());
		}

		if !active && self.pointer_queue.is_some() {
			self.pointer_queue.take();
			let Ok(focus) = self.focus.upgrade() else {return};
			let Some((pointer, pointer_focus)) = seat_data.pointer.get() else {return};
			if &*pointer_focus.lock() == &Some(self.toplevel.id()) {
				pointer.leave(SERIAL_COUNTER.inc(), &focus);
			}
		}
	}
	fn handle_pointer_events(&mut self, pointer: &WlPointer, mut locked: bool) -> bool {
		let Ok(focus) = self.focus.upgrade() else { return false; };
		let Some(pointer_queue) = self.pointer_queue.as_mut() else { return false; };
		let Some(core_surface) = CoreSurface::from_wl_surface(&focus) else { return false; };
		let Some(focus_size) = core_surface.size() else { return false; };

		if !pointer_queue.is_empty() {
			self.pointer_latest_event = Instant::now();
		}
		while let Some(event) = pointer_queue.pop_front() {
			match (locked, event) {
				(false, PointerEvent::Motion(pos)) => {
					pointer.enter(
						SERIAL_COUNTER.inc(),
						&focus,
						pos.x.clamp(0.0, focus_size.x as f64),
						pos.y.clamp(0.0, focus_size.y as f64),
					);
					locked = true;
				}
				(true, PointerEvent::Motion(pos)) => {
					pointer.motion(
						0,
						pos.x.clamp(0.0, focus_size.x as f64),
						pos.y.clamp(0.0, focus_size.y as f64),
					);
					pointer.frame();
				}
				(true, PointerEvent::Button { button, state }) => {
					pointer.button(
						0,
						0,
						button,
						match state {
							0 => ButtonState::Released,
							1 => ButtonState::Pressed,
							_ => continue,
						},
					);
					pointer.frame();
				}
				(
					true,
					PointerEvent::Scroll {
						axis_continuous,
						axis_discrete,
					},
				) => {
					if let Some(axis_continuous) = axis_continuous {
						pointer.axis(0, Axis::HorizontalScroll, axis_continuous.x as f64);
						pointer.axis(0, Axis::VerticalScroll, axis_continuous.y as f64);
					}
					if let Some(axis_discrete) = axis_discrete {
						pointer.axis_discrete(Axis::HorizontalScroll, axis_discrete.x as i32);
						pointer.axis_discrete(Axis::VerticalScroll, axis_discrete.y as i32);
					}
					if axis_discrete.is_none() && axis_continuous.is_none() {
						pointer.axis_stop(0, Axis::HorizontalScroll);
						pointer.axis_stop(0, Axis::VerticalScroll);
					}
					pointer.frame();
				}
				(locked, event) => {
					warn!(locked, ?event, "Invalid pointer event!");
				}
			}
		}
		if self.pointer_latest_event.elapsed() > POINTER_EVENT_TIMEOUT {
			pointer.leave(SERIAL_COUNTER.inc(), &focus);
			locked = false;
		}

		locked
	}
	pub fn set_keyboard_active(&mut self, seat_data: &SeatDataInner, active: bool) {
		if active && self.keyboard_queue.is_none() {
			self.keyboard_queue.replace(Default::default());
		}
		if !active && self.keyboard_queue.is_some() {
			self.keyboard_queue.take();
			let Ok(focus) = self.focus.upgrade() else {return};
			let Some((keyboard, keyboard_focus)) = seat_data.keyboard.get() else {return};
			if &*keyboard_focus.lock() == &Some(self.toplevel.id()) {
				keyboard.leave(SERIAL_COUNTER.inc(), &focus);
			}
		}
	}
	fn handle_keyboard_events(&mut self, keyboard: &WlKeyboard, mut locked: bool) -> bool {
		let Ok(focus) = self.focus.upgrade() else { return false; };
		let Some(keyboard_queue) = self.keyboard_queue.as_mut() else { return false; };
		let Some(info) = self.keyboard_info.as_mut() else { return true; };

		if !locked {
			keyboard.enter(0, &focus, vec![]);
			keyboard.repeat_info(0, 0);
			locked = info.keymap.send(keyboard).is_ok();
		}
		while let Some(event) = keyboard_queue.pop_front() {
			debug!(locked, ?event, "Process keyboard event");
			match (locked, event) {
				(true, KeyboardEvent::Keymap) => {
					let _ = info.keymap.send(keyboard);
				}
				(true, KeyboardEvent::Key { key, state }) => {
					if let Ok(key_count) = info.process(key, state, keyboard) {
						if key_count == 0 {
							keyboard.leave(SERIAL_COUNTER.inc(), &focus);
							return false;
						}
					}
				}
				(locked, event) => {
					warn!(locked, ?event, "Invalid keyboard event!");
				}
			}
		}
		locked
	}
}

pub struct SeatDataInner {
	client: ClientId,
	global_id: OnceCell<GlobalId>,
	panels: Mutex<FxHashMap<ObjectId, PanelInfo>>,
	pointer: OnceCell<(WlPointer, Mutex<Option<ObjectId>>)>,
	keyboard: OnceCell<(WlKeyboard, Mutex<Option<ObjectId>>)>,
	touch: OnceCell<WlTouch>,
}
impl SeatDataInner {
	// pub fn set_focus(&self, toplevel: &WlSurface, focus: &WlSurface) {
	// 	if let Some(panel_info) = self.panels.lock().get_mut(&toplevel.id()) {
	// 		panel_info.focus = focus.downgrade();
	// 		match panel_info.pointer_queue.back() {
	// 			None => (),
	// 			Some(&PointerEvent::Leave) => (),
	// 			_ => panel_info.pointer_queue.push_back(PointerEvent::Leave),
	// 		};
	// 		match panel_info.keyboard_queue.back() {
	// 			None => (),
	// 			Some(&KeyboardEvent::Leave) => (),
	// 			_ => panel_info.keyboard_queue.push_back(KeyboardEvent::Leave),
	// 		};
	// 	}
	// }
	pub fn set_pointer_active(&self, toplevel: &XdgToplevel, active: bool) {
		let mut panels = self.panels.lock();
		let Some(panel_info) = panels.get_mut(&toplevel.id()) else {return};
		panel_info.set_pointer_active(self, active);
	}
	pub fn set_keyboard_active(&self, toplevel: &XdgToplevel, active: bool) {
		let mut panels = self.panels.lock();
		let Some(panel_info) = panels.get_mut(&toplevel.id()) else {return};
		panel_info.set_keyboard_active(self, active);
	}
	pub fn set_keymap(&self, toplevel: &XdgToplevel, keymap: &Keymap) {
		let mut panels = self.panels.lock();
		let Some(panel_info) = panels.get_mut(&toplevel.id()) else {return};
		panel_info.keyboard_info.replace(KeyboardInfo::new(keymap));

		let Some(keyboard_queue) = panel_info.keyboard_queue.as_mut() else {return};
		let Some((_, focus)) = self.keyboard.get() else {return};
		let Some(id) = &*focus.lock() else {return};
		if id == &toplevel.id() {
			keyboard_queue.push_back(KeyboardEvent::Keymap);
		}
	}

	pub fn pointer_event(&self, toplevel: &XdgToplevel, event: PointerEvent) {
		let mut panels = self.panels.lock();
		let Some(panel_info) = panels.get_mut(&toplevel.id()) else {return};
		let Some(pointer_queue) = panel_info.pointer_queue.as_mut() else {return};
		pointer_queue.push_back(event);
		drop(panels);
		self.handle_pointer_events();
	}
	pub fn keyboard_event(&self, toplevel: &XdgToplevel, event: KeyboardEvent) {
		let mut panels = self.panels.lock();
		let Some(panel_info) = panels.get_mut(&toplevel.id()) else {return};
		let Some(keyboard_queue) = panel_info.keyboard_queue.as_mut() else {return};
		keyboard_queue.push_back(event);
		drop(panels);
		self.handle_keyboard_events();
	}

	fn handle_pointer_events(&self) {
		let mut panels = self.panels.lock();
		let Some((pointer, pointer_focus)) = self.pointer.get() else {return};
		let mut pointer_focus = pointer_focus.lock();

		loop {
			let locked = pointer_focus.is_some();
			// Pick a pointer to focus on if there is none
			if pointer_focus.is_none() {
				*pointer_focus = panels
					.iter()
					.filter(|(_k, v)| v.pointer_queue.is_some())
					.filter(|(_k, v)| !v.pointer_queue.as_ref().unwrap().is_empty())
					.map(|(k, _v)| k)
					.choose(&mut thread_rng())
					.cloned();
			}
			if pointer_focus.is_none() {
				// If there's still none, guess we're done with pointer events for the time being
				break;
			}
			let Some(panel_info) = panels.get_mut(pointer_focus.as_ref().unwrap()) else {break};
			if panel_info.handle_pointer_events(pointer, locked) {
				// We haven't gotten to a point where we can switch the focus
				break;
			} else {
				pointer_focus.take();
			}
		}
	}
	fn handle_keyboard_events(&self) {
		let mut panels = self.panels.lock();
		let Some((keyboard, keyboard_focus)) = self.keyboard.get() else {return};
		let mut keyboard_focus = keyboard_focus.lock();
		loop {
			let locked = keyboard_focus.is_some();
			// Pick a keyboard to focus on if there is none
			if keyboard_focus.is_none() {
				*keyboard_focus = panels
					.iter()
					.filter(|(_k, v)| v.keyboard_info.is_some())
					.filter(|(_k, v)| v.keyboard_queue.is_some())
					.filter(|(_k, v)| !v.keyboard_queue.as_ref().unwrap().is_empty())
					.map(|(k, _v)| k)
					.choose(&mut thread_rng())
					.cloned();
			}
			if keyboard_focus.is_none() {
				// If there's still none, guess we're done with keyboard events for the time being
				break;
			}
			let Some(panel_info) = panels.get_mut(keyboard_focus.as_ref().unwrap()) else {break};
			if panel_info.handle_keyboard_events(keyboard, locked) {
				// We haven't gotten to a point where we can switch the focus
				break;
			} else {
				keyboard_focus.take();
			}
		}
	}

	pub fn new_panel_item(
		&self,
		panel_item: &Arc<PanelItem>,
		toplevel: &XdgToplevel,
		focus: &WlSurface,
	) {
		self.panels
			.lock()
			.insert(toplevel.id(), PanelInfo::new(panel_item, toplevel, focus));
	}
	pub fn drop_panel_item(&self, toplevel: &XdgToplevel) {
		self.panels.lock().remove(&toplevel.id());
		if let Some((_, pointer_focus)) = self.pointer.get() {
			let mut pointer_focus = pointer_focus.lock();
			if *pointer_focus == Some(toplevel.id()) {
				pointer_focus.take();
			}
		}
		if let Some((_, keyboard_focus)) = self.keyboard.get() {
			let mut keyboard_focus = keyboard_focus.lock();
			if *keyboard_focus == Some(toplevel.id()) {
				keyboard_focus.take();
			}
		}
	}
}
impl Drop for SeatDataInner {
	fn drop(&mut self) {
		let id = self.global_id.take().unwrap();
		let _ = task::new(|| "global destroy queue garbage collection", async move {
			GLOBAL_DESTROY_QUEUE.get().unwrap().send(id).await
		});
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
		request: wl_seat::Request,
		data: &SeatData,
		_dh: &DisplayHandle,
		data_init: &mut DataInit<'_, WaylandState>,
	) {
		match request {
			wl_seat::Request::GetPointer { id } => {
				let pointer = data_init.init(id, data.clone());
				let _ = data.0.pointer.set((pointer, Mutex::new(None)));
			}
			wl_seat::Request::GetKeyboard { id } => {
				let keyboard = data_init.init(id, data.clone());
				keyboard.repeat_info(0, 0);
				let _ = data.0.keyboard.set((keyboard, Mutex::new(None)));
			}
			wl_seat::Request::GetTouch { id } => {
				let _ = data.0.touch.set(data_init.init(id, data.clone()));
			}
			wl_seat::Request::Release => (),
			_ => unreachable!(),
		}
	}
}

pub struct Cursor {
	pub hotspot: Vector2<i32>,
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

				let Some((_, focus)) = seat_data.pointer.get() else {return};
				let focus = focus.lock();
				let Some(id) = &*focus else {return};
				let panels = seat_data.panels.lock();
				let Some(panel_info) = panels.get(&id) else {return};
				let Some(panel_item) = panel_info.panel_item.upgrade() else {return};
				panel_item.set_cursor(surface.as_ref(), hotspot_x, hotspot_y);
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

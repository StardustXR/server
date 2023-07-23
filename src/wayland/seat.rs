use super::{
	panel_item::PanelItem, state::WaylandState, surface::CoreSurface, GLOBAL_DESTROY_QUEUE,
	SERIAL_COUNTER,
};
use crate::core::task;
use color_eyre::eyre::Result;
use mint::Vector2;
use nanoid::nanoid;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use rand::{seq::IteratorRandom, thread_rng};
use rustc_hash::{FxHashMap, FxHashSet};
use smithay::{
	input::keyboard::{KeymapFile, ModifiersState},
	reexports::wayland_server::{
		backend::{ClientId, GlobalId, ObjectId},
		protocol::{
			wl_keyboard::{self, KeyState, WlKeyboard},
			wl_pointer::{self, Axis, ButtonState, WlPointer},
			wl_seat::{self, Capability, WlSeat, EVT_NAME_SINCE},
			wl_surface::WlSurface,
			wl_touch::{self, WlTouch},
		},
		Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, Weak as WlWeak,
	},
	wayland::compositor,
};
use std::{
	collections::VecDeque,
	sync::{Arc, Weak},
	time::{Duration, Instant},
};
use tracing::{debug, warn};
use xkbcommon::xkb::{self, Keymap};

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
			keymap: KeymapFile::new(keymap),
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
struct SurfaceInfo {
	wl_surface: WlWeak<WlSurface>,
	panel_item: Weak<PanelItem>,
	pointer_queue: VecDeque<PointerEvent>,
	pointer_latest_event: Instant,
	keyboard_queue: VecDeque<KeyboardEvent>,
	keyboard_info: Option<KeyboardInfo>,
}
impl SurfaceInfo {
	fn new(wl_surface: &WlSurface, panel_item: Weak<PanelItem>) -> Self {
		SurfaceInfo {
			wl_surface: wl_surface.downgrade(),
			panel_item,
			pointer_queue: VecDeque::new(),
			pointer_latest_event: Instant::now(),
			keyboard_queue: VecDeque::new(),
			keyboard_info: None,
		}
	}
	fn handle_pointer_events(&mut self, pointer: &WlPointer, mut locked: bool) -> bool {
		let Ok(focus) = self.wl_surface.upgrade() else { return false; };
		let Some(core_surface) = CoreSurface::from_wl_surface(&focus) else { return false; };
		let Some(focus_size) = core_surface.size() else { return false; };

		if !self.pointer_queue.is_empty() {
			self.pointer_latest_event = Instant::now();
		}
		while let Some(event) = self.pointer_queue.pop_front() {
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
					if pointer.version() >= wl_pointer::EVT_FRAME_SINCE {
						pointer.frame();
					}
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
					if pointer.version() >= wl_pointer::EVT_FRAME_SINCE {
						pointer.frame();
					}
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
					if pointer.version() >= wl_pointer::EVT_AXIS_DISCRETE_SINCE {
						if let Some(axis_discrete) = axis_discrete {
							pointer.axis_discrete(Axis::HorizontalScroll, axis_discrete.x as i32);
							pointer.axis_discrete(Axis::VerticalScroll, axis_discrete.y as i32);
						}
					}
					if pointer.version() >= wl_pointer::EVT_AXIS_STOP_SINCE
						&& axis_discrete.is_none()
						&& axis_continuous.is_none()
					{
						pointer.axis_stop(0, Axis::HorizontalScroll);
						pointer.axis_stop(0, Axis::VerticalScroll);
					}
					if pointer.version() >= wl_pointer::EVT_FRAME_SINCE {
						pointer.frame();
					}
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
	fn handle_keyboard_events(&mut self, keyboard: &WlKeyboard, mut locked: bool) -> bool {
		let Ok(focus) = self.wl_surface.upgrade() else { return false; };
		let Some(info) = self.keyboard_info.as_mut() else { return true; };

		if !locked {
			keyboard.enter(0, &focus, vec![]);
			if keyboard.version() >= wl_keyboard::EVT_REPEAT_INFO_SINCE {
				keyboard.repeat_info(0, 0);
			}
			locked = info.keymap.send(keyboard).is_ok();
		}
		while let Some(event) = self.keyboard_queue.pop_front() {
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

pub struct SeatData {
	client: ClientId,
	global_id: OnceCell<GlobalId>,
	surfaces: Mutex<FxHashMap<ObjectId, SurfaceInfo>>,
	pointer: OnceCell<(WlPointer, Mutex<ObjectId>)>,
	keyboard: OnceCell<(WlKeyboard, Mutex<ObjectId>)>,
	touch: OnceCell<WlTouch>,
}
impl SeatData {
	pub fn new(dh: &DisplayHandle, client: ClientId) -> Arc<Self> {
		let seat_data = Arc::new(SeatData {
			client,
			global_id: OnceCell::new(),
			surfaces: Mutex::new(FxHashMap::default()),
			pointer: OnceCell::new(),
			keyboard: OnceCell::new(),
			touch: OnceCell::new(),
		});

		seat_data
			.global_id
			.set(dh.create_global::<WaylandState, _, _>(7, seat_data.clone()))
			.unwrap();

		seat_data
	}

	pub fn set_keymap(&self, keymap: &Keymap, surfaces: Vec<WlSurface>) {
		let mut panels = self.surfaces.lock();
		let Some((_, focus)) = self.keyboard.get() else {return};
		for surface in surfaces {
			let Some(surface_info) = panels.get_mut(&surface.id()) else {continue};
			surface_info
				.keyboard_info
				.replace(KeyboardInfo::new(keymap));

			if *focus.lock() == surface.id() {
				surface_info.keyboard_queue.push_back(KeyboardEvent::Keymap);
			}
		}
	}

	pub fn pointer_event(&self, surface: &WlSurface, event: PointerEvent) {
		let mut surfaces = self.surfaces.lock();
		let Some(surface_info) = surfaces.get_mut(&surface.id()) else {return};
		surface_info.pointer_queue.push_back(event);
		drop(surfaces);
		self.handle_pointer_events();
	}
	pub fn keyboard_event(&self, surface: &WlSurface, event: KeyboardEvent) {
		let mut surfaces = self.surfaces.lock();
		let Some(surface_info) = surfaces.get_mut(&surface.id()) else {return};
		surface_info.keyboard_queue.push_back(event);
		drop(surfaces);
		self.handle_keyboard_events();
	}

	fn handle_pointer_events(&self) {
		let mut surfaces = self.surfaces.lock();
		let Some((pointer, pointer_focus)) = self.pointer.get() else {return};
		let mut pointer_focus = pointer_focus.lock();

		loop {
			let locked = !pointer_focus.is_null();
			// Pick a pointer to focus on if there is none
			if pointer_focus.is_null() {
				*pointer_focus = surfaces
					.iter()
					.filter(|(_k, v)| !v.pointer_queue.is_empty())
					.map(|(k, _v)| k)
					.choose(&mut thread_rng())
					.cloned()
					.unwrap_or(ObjectId::null());
			}
			if pointer_focus.is_null() {
				// If there's still none, guess we're done with pointer events for the time being
				break;
			}
			let Some(surface_info) = surfaces.get_mut(&pointer_focus) else {break};
			if surface_info.handle_pointer_events(pointer, locked) {
				// We haven't gotten to a point where we can switch the focus
				break;
			} else {
				*pointer_focus = ObjectId::null();
			}
		}
	}
	fn handle_keyboard_events(&self) {
		let mut surfaces = self.surfaces.lock();
		let Some((keyboard, keyboard_focus)) = self.keyboard.get() else {return};
		let mut keyboard_focus = keyboard_focus.lock();
		loop {
			let locked = !keyboard_focus.is_null();
			// Pick a keyboard to focus on if there is none
			if keyboard_focus.is_null() {
				*keyboard_focus = surfaces
					.iter()
					.filter(|(_k, v)| v.keyboard_info.is_some())
					.filter(|(_k, v)| !v.keyboard_queue.is_empty())
					.map(|(k, _v)| k)
					.choose(&mut thread_rng())
					.cloned()
					.unwrap_or(ObjectId::null());
			}
			// If there's still none, guess we're done with keyboard events for the time being
			let Some(surface_info) = surfaces.get_mut(&keyboard_focus) else {break};
			if surface_info.handle_keyboard_events(keyboard, locked) {
				// We haven't gotten to a point where we can switch the focus
				break;
			} else {
				*keyboard_focus = ObjectId::null();
			}
		}
	}

	pub fn new_surface(&self, surface: &WlSurface, panel_item: Weak<PanelItem>) {
		self.surfaces
			.lock()
			.insert(surface.id(), SurfaceInfo::new(surface, panel_item));
	}
	pub fn drop_surface(&self, surface: &WlSurface) {
		self.surfaces.lock().remove(&surface.id());
		if let Some((_, pointer_focus)) = self.pointer.get() {
			let mut pointer_focus = pointer_focus.lock();
			if *pointer_focus == surface.id() {
				*pointer_focus = ObjectId::null();
			}
		}
		if let Some((_, keyboard_focus)) = self.keyboard.get() {
			let mut keyboard_focus = keyboard_focus.lock();
			if *keyboard_focus == surface.id() {
				*keyboard_focus = ObjectId::null();
			}
		}
	}
}
impl Drop for SeatData {
	fn drop(&mut self) {
		let id = self.global_id.take().unwrap();
		let _ = task::new(|| "global destroy queue garbage collection", async move {
			GLOBAL_DESTROY_QUEUE.get().unwrap().send(id).await
		});
	}
}

impl GlobalDispatch<WlSeat, Arc<SeatData>, WaylandState> for WaylandState {
	fn bind(
		_state: &mut WaylandState,
		_handle: &DisplayHandle,
		_client: &Client,
		resource: New<WlSeat>,
		data: &Arc<SeatData>,
		data_init: &mut DataInit<'_, WaylandState>,
	) {
		let resource = data_init.init(resource, data.clone());

		if resource.version() >= EVT_NAME_SINCE {
			resource.name(nanoid!());
		}

		resource.capabilities(Capability::Pointer | Capability::Keyboard);
	}

	fn can_view(client: Client, data: &Arc<SeatData>) -> bool {
		client.id() == data.client
	}
}

impl Dispatch<WlSeat, Arc<SeatData>, WaylandState> for WaylandState {
	fn request(
		_state: &mut WaylandState,
		_client: &Client,
		_resource: &WlSeat,
		request: wl_seat::Request,
		data: &Arc<SeatData>,
		_dh: &DisplayHandle,
		data_init: &mut DataInit<'_, WaylandState>,
	) {
		match request {
			wl_seat::Request::GetPointer { id } => {
				let pointer = data_init.init(id, data.clone());
				let _ = data.pointer.set((pointer, Mutex::new(ObjectId::null())));
			}
			wl_seat::Request::GetKeyboard { id } => {
				let keyboard = data_init.init(id, data.clone());
				if keyboard.version() >= wl_keyboard::EVT_REPEAT_INFO_SINCE {
					keyboard.repeat_info(0, 0);
				}
				let _ = data.keyboard.set((keyboard, Mutex::new(ObjectId::null())));
			}
			wl_seat::Request::GetTouch { id } => {
				let _ = data.touch.set(data_init.init(id, data.clone()));
			}
			wl_seat::Request::Release => (),
			_ => unreachable!(),
		}
	}
}

pub struct Cursor {
	pub hotspot: Vector2<i32>,
}
impl Dispatch<WlPointer, Arc<SeatData>, WaylandState> for WaylandState {
	fn request(
		state: &mut WaylandState,
		_client: &Client,
		_resource: &WlPointer,
		request: wl_pointer::Request,
		seat_data: &Arc<SeatData>,
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
					CoreSurface::add_to(&state.display, dh.clone(), surface, || (), |_| ());
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
				let surfaces = seat_data.surfaces.lock();
				let Some(surface_info) = surfaces.get(&focus) else {return};
				let Some(panel_item) = surface_info.panel_item.upgrade() else {return};
				panel_item.set_cursor(surface.as_ref(), hotspot_x, hotspot_y);
			}
			wl_pointer::Request::Release => (),
			_ => unreachable!(),
		}
	}
}

impl Dispatch<WlKeyboard, Arc<SeatData>, WaylandState> for WaylandState {
	fn request(
		_state: &mut WaylandState,
		_client: &Client,
		_resource: &WlKeyboard,
		request: <WlKeyboard as Resource>::Request,
		_data: &Arc<SeatData>,
		_dh: &DisplayHandle,
		_data_init: &mut DataInit<'_, WaylandState>,
	) {
		match request {
			wl_keyboard::Request::Release => (),
			_ => unreachable!(),
		}
	}
}

impl Dispatch<WlTouch, Arc<SeatData>, WaylandState> for WaylandState {
	fn request(
		_state: &mut WaylandState,
		_client: &Client,
		_resource: &WlTouch,
		request: <WlTouch as Resource>::Request,
		_data: &Arc<SeatData>,
		_dh: &DisplayHandle,
		_data_init: &mut DataInit<'_, WaylandState>,
	) {
		match request {
			wl_touch::Request::Release => (),
			_ => unreachable!(),
		}
	}
}

use super::{
	state::{ClientState, WaylandState},
	surface::CoreSurface,
	GLOBAL_DESTROY_QUEUE, SERIAL_COUNTER,
};
use crate::{
	core::task,
	nodes::items::panel::{Backend, Geometry, PanelItem},
};
use color_eyre::eyre::Result;
use mint::Vector2;
use nanoid::nanoid;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use rand::{seq::IteratorRandom, thread_rng};
use rustc_hash::{FxHashMap, FxHashSet};
use smithay::{
	input::keyboard::ModifiersState,
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
	sync::Arc,
	time::{Duration, Instant},
};
use tokio::sync::watch;
use tracing::{debug, warn};
use xkbcommon::xkb::{self, Context, Keymap};

pub fn handle_cursor<B: Backend>(
	panel_item: &Arc<PanelItem<B>>,
	mut cursor: watch::Receiver<Option<CursorInfo>>,
) {
	let panel_item_weak = Arc::downgrade(panel_item);
	let _ = task::new(|| "cursor handler", async move {
		while cursor.changed().await.is_ok() {
			let Some(panel_item) = panel_item_weak.upgrade() else {continue};
			let cursor_info = cursor.borrow();
			panel_item.set_cursor(cursor_info.as_ref().and_then(CursorInfo::cursor_data));
		}
	});
}

pub struct KeyboardInfo {
	state: xkb::State,
	mods: ModifiersState,
	keys: FxHashSet<u32>,
}
impl KeyboardInfo {
	pub fn new(keymap: &Keymap) -> Self {
		KeyboardInfo {
			state: xkb::State::new(keymap),
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
	Motion(Vector2<f32>),
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
	Key { key: u32, state: u32 },
}

const POINTER_EVENT_TIMEOUT: Duration = Duration::from_secs(1);
struct SurfaceInfo {
	wl_surface: WlWeak<WlSurface>,
	cursor_sender: watch::Sender<Option<CursorInfo>>,
	pointer_queue: VecDeque<PointerEvent>,
	pointer_latest_event: Instant,
	keyboard_queue: VecDeque<KeyboardEvent>,
	keyboard_info: KeyboardInfo,
}
impl SurfaceInfo {
	fn new(wl_surface: &WlSurface, cursor_sender: watch::Sender<Option<CursorInfo>>) -> Self {
		SurfaceInfo {
			wl_surface: wl_surface.downgrade(),
			cursor_sender,
			pointer_queue: VecDeque::new(),
			pointer_latest_event: Instant::now(),
			keyboard_queue: VecDeque::new(),
			keyboard_info: KeyboardInfo::new(
				&Keymap::new_from_names(&Context::new(0), "", "", "", "", None, 0).unwrap(),
			),
		}
	}
	fn flush(&self) {
		if let Some(client) = self.wl_surface.upgrade().ok().and_then(|s| s.client()) {
			if let Some(client_data) = client.get_data::<ClientState>() {
				client_data.flush();
			}
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
						(pos.x as f64).clamp(0.0, focus_size.x as f64),
						(pos.y as f64).clamp(0.0, focus_size.y as f64),
					);
					locked = true;
				}
				(true, PointerEvent::Motion(pos)) => {
					pointer.motion(
						0,
						(pos.x as f64).clamp(0.0, focus_size.x as f64),
						(pos.y as f64).clamp(0.0, focus_size.y as f64),
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
		self.flush();

		locked
	}
	fn handle_keyboard_events(&mut self, keyboard: &WlKeyboard, locked: bool) -> bool {
		let Ok(focus) = self.wl_surface.upgrade() else { return false; };

		if !locked {
			keyboard.enter(0, &focus, vec![]);
			if keyboard.version() >= wl_keyboard::EVT_REPEAT_INFO_SINCE {
				keyboard.repeat_info(0, 0);
			}
		}
		while let Some(event) = self.keyboard_queue.pop_front() {
			debug!(locked, ?event, "Process keyboard event");
			match (locked, event) {
				(true, KeyboardEvent::Key { key, state }) => {
					if let Ok(key_count) = self.keyboard_info.process(key, state, keyboard) {
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
		self.flush();
		locked
	}
}

pub struct SeatData {
	pub client: OnceCell<ClientId>,
	global_id: OnceCell<GlobalId>,
	surfaces: Mutex<FxHashMap<ObjectId, SurfaceInfo>>,
	pointer: OnceCell<(WlPointer, Mutex<ObjectId>)>,
	keyboard: OnceCell<(WlKeyboard, Mutex<ObjectId>)>,
	touch: OnceCell<WlTouch>,
}
impl SeatData {
	pub fn new(dh: &DisplayHandle) -> Arc<Self> {
		let seat_data = Arc::new(SeatData {
			client: OnceCell::new(),
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

	pub fn new_surface(&self, surface: &WlSurface) -> watch::Receiver<Option<CursorInfo>> {
		let (tx, rx) = watch::channel(None);
		self.surfaces
			.lock()
			.insert(surface.id(), SurfaceInfo::new(surface, tx));

		rx
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

pub struct CursorInfo {
	pub surface: WlWeak<WlSurface>,
	pub hotspot_x: i32,
	pub hotspot_y: i32,
}
impl CursorInfo {
	pub fn cursor_data(&self) -> Option<Geometry> {
		let cursor_size = CoreSurface::from_wl_surface(&self.surface.upgrade().ok()?)?.size()?;
		Some(Geometry {
			origin: [self.hotspot_x, self.hotspot_y].into(),
			size: cursor_size,
		})
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
		let Some(seat_client) = data.client.get().cloned() else {return false};
		client.id() == seat_client
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

impl Dispatch<WlPointer, Arc<SeatData>, WaylandState> for WaylandState {
	fn request(
		_state: &mut WaylandState,
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
					CoreSurface::add_to(dh.clone(), surface, || (), |_| ());
					compositor::with_states(surface, |data| {
						if let Some(core_surface) = data.data_map.get::<Arc<CoreSurface>>() {
							core_surface.set_material_offset(1);
						}
					})
				}

				let Some((_, focus)) = seat_data.pointer.get() else {return};
				let focus = focus.lock();
				let surfaces = seat_data.surfaces.lock();
				let Some(surface_info) = surfaces.get(&focus) else {return};
				let cursor_info = surface.map(|surface| CursorInfo {
					surface: surface.downgrade(),
					hotspot_x,
					hotspot_y,
				});
				let _ = surface_info.cursor_sender.send_replace(cursor_info);
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

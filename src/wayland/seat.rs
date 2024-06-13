use super::{state::WaylandState, surface::CoreSurface};
use crate::{
	core::task,
	nodes::{
		data::KEYMAPS,
		items::panel::{Backend, Geometry, PanelItem},
	},
};
use mint::Vector2;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use slotmap::KeyData;
use smithay::{
	backend::input::{AxisRelativeDirection, ButtonState, KeyState},
	delegate_seat,
	input::{
		keyboard::{FilterResult, LedState},
		pointer::{AxisFrame, ButtonEvent, CursorImageStatus, MotionEvent},
		touch::{self, DownEvent, UpEvent},
		Seat, SeatHandler,
	},
	reexports::wayland_server::{protocol::wl_surface::WlSurface, Resource, Weak as WlWeak},
	utils::SERIAL_COUNTER,
	wayland::compositor,
};
use std::sync::{Arc, Weak};
use tokio::sync::watch;

impl SeatHandler for WaylandState {
	type PointerFocus = WlSurface;
	type KeyboardFocus = WlSurface;
	type TouchFocus = WlSurface;

	fn seat_state(&mut self) -> &mut smithay::input::SeatState<Self> {
		&mut self.seat_state
	}
	fn focus_changed(&mut self, _seat: &Seat<Self>, _focused: Option<&Self::KeyboardFocus>) {}
	fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
		self.seat.cursor_info_tx.send_modify(|c| match image {
			CursorImageStatus::Hidden => c.surface = None,
			CursorImageStatus::Surface(surface) => {
				CoreSurface::add_to(&surface, || (), |_| ());
				compositor::with_states(&surface, |data| {
					if let Some(core_surface) = data.data_map.get::<Arc<CoreSurface>>() {
						core_surface.set_material_offset(1);
					}
				});
				c.surface = Some(surface.downgrade())
			}
			_ => (),
		});
	}
	fn led_state_changed(&mut self, _seat: &Seat<Self>, _led_state: LedState) {}
}
delegate_seat!(WaylandState);

pub fn handle_cursor<B: Backend>(
	panel_item: &Arc<PanelItem<B>>,
	mut cursor: watch::Receiver<CursorInfo>,
) {
	let panel_item_weak = Arc::downgrade(panel_item);
	let _ = task::new(|| "cursor handler", async move {
		while cursor.changed().await.is_ok() {
			let Some(panel_item) = panel_item_weak.upgrade() else {
				continue;
			};
			let cursor_info = cursor.borrow();
			panel_item.set_cursor(cursor_info.cursor_data());
		}
	});
}
pub struct CursorInfo {
	pub surface: Option<WlWeak<WlSurface>>,
	pub hotspot_x: i32,
	pub hotspot_y: i32,
}
impl CursorInfo {
	pub fn cursor_data(&self) -> Option<Geometry> {
		let cursor_size =
			CoreSurface::from_wl_surface(&self.surface.as_ref()?.upgrade().ok()?)?.size()?;
		Some(Geometry {
			origin: [self.hotspot_x, self.hotspot_y].into(),
			size: cursor_size,
		})
	}
}

pub struct SeatWrapper {
	wayland_state: Weak<Mutex<WaylandState>>,
	cursor_info_tx: watch::Sender<CursorInfo>,
	pub cursor_info_rx: watch::Receiver<CursorInfo>,
	seat: Seat<WaylandState>,
	touches: Mutex<FxHashMap<u32, WlWeak<WlSurface>>>,
}
impl SeatWrapper {
	pub fn new(wayland_state: Weak<Mutex<WaylandState>>, seat: Seat<WaylandState>) -> Self {
		let (cursor_info_tx, cursor_info_rx) = watch::channel(CursorInfo {
			surface: None,
			hotspot_x: 0,
			hotspot_y: 0,
		});
		SeatWrapper {
			wayland_state,
			cursor_info_tx,
			cursor_info_rx,
			seat,
			touches: Mutex::new(FxHashMap::default()),
		}
	}
	pub fn unfocus(&self, surface: &WlSurface, state: &mut WaylandState) {
		let pointer = self.seat.get_pointer().unwrap();
		if pointer.current_focus() == Some(surface.clone()) {
			pointer.motion(
				state,
				None,
				&MotionEvent {
					location: (0.0, 0.0).into(),
					serial: SERIAL_COUNTER.next_serial(),
					time: 0,
				},
			)
		}
		let keyboard = self.seat.get_keyboard().unwrap();
		if keyboard.current_focus() == Some(surface.clone()) {
			keyboard.set_focus(state, None, SERIAL_COUNTER.next_serial());
		}
		let touch = self.seat.get_touch().unwrap();
		for (id, touch_surface) in self.touches.lock().iter() {
			if touch_surface.id() == surface.id() {
				self.touch_up(*id);
				touch.up(
					state,
					&UpEvent {
						slot: Some(*id).into(),
						serial: SERIAL_COUNTER.next_serial(),
						time: 0,
					},
				)
			}
		}
	}

	pub fn pointer_motion(&self, surface: WlSurface, position: Vector2<f32>) {
		let Some(state) = self.wayland_state.upgrade() else {
			return;
		};
		let mut state = state.lock();
		let Some(pointer) = self.seat.get_pointer() else {
			return;
		};
		pointer.motion(
			&mut state,
			Some((surface, (0, 0).into())),
			&MotionEvent {
				location: (position.x as f64, position.y as f64).into(),
				serial: SERIAL_COUNTER.next_serial(),
				time: 0,
			},
		);
		pointer.frame(&mut state);
	}
	pub fn pointer_button(&self, button: u32, pressed: bool) {
		let Some(state) = self.wayland_state.upgrade() else {
			return;
		};
		let mut state = state.lock();
		let Some(pointer) = self.seat.get_pointer() else {
			return;
		};
		pointer.button(
			&mut state,
			&ButtonEvent {
				button,
				state: if pressed {
					ButtonState::Pressed
				} else {
					ButtonState::Released
				},
				serial: SERIAL_COUNTER.next_serial(),
				time: 0,
			},
		);
		pointer.frame(&mut state);
	}
	pub fn pointer_scroll(
		&self,
		scroll_distance: Option<Vector2<f32>>,
		scroll_steps: Option<Vector2<f32>>,
	) {
		let Some(state) = self.wayland_state.upgrade() else {
			return;
		};
		let mut state = state.lock();
		let Some(pointer) = self.seat.get_pointer() else {
			return;
		};
		pointer.axis(
			&mut state,
			AxisFrame {
				source: None,
				relative_direction: (
					AxisRelativeDirection::Identical,
					AxisRelativeDirection::Identical,
				),
				time: 0,
				axis: scroll_distance
					.map(|d| (d.x as f64, d.y as f64))
					.unwrap_or((0.0, 0.0)),
				v120: scroll_steps.map(|d| ((d.x * 120.0) as i32, (d.y * 120.0) as i32)),
				stop: (false, false),
			},
		);
		pointer.frame(&mut state);
	}

	pub fn keyboard_keys(&self, surface: WlSurface, keymap_id: u64, keys: Vec<i32>) {
		let Some(state) = self.wayland_state.upgrade() else {
			return;
		};
		let Some(keyboard) = self.seat.get_keyboard() else {
			return;
		};
		let keymaps = KEYMAPS.lock();
		let Some(keymap) = keymaps.get(KeyData::from_ffi(keymap_id).into()).cloned() else {
			return;
		};

		keyboard.set_focus(
			&mut state.lock(),
			Some(surface),
			SERIAL_COUNTER.next_serial(),
		);
		if keyboard
			.set_keymap_from_string(&mut state.lock(), keymap)
			.is_err()
		{
			return;
		}
		for key in keys {
			keyboard.input(
				&mut state.lock(),
				key.unsigned_abs(),
				if key > 0 {
					KeyState::Pressed
				} else {
					KeyState::Released
				},
				SERIAL_COUNTER.next_serial(),
				0,
				|_, _, _| FilterResult::Forward::<()>,
			);
		}
	}

	pub fn touch_down(&self, surface: WlSurface, id: u32, position: Vector2<f32>) {
		let Some(state) = self.wayland_state.upgrade() else {
			return;
		};
		let Some(touch) = self.seat.get_touch() else {
			return;
		};
		touch.down(
			&mut state.lock(),
			Some((surface, (0, 0).into())),
			&DownEvent {
				slot: Some(id).into(),
				location: (position.x as f64, position.y as f64).into(),
				serial: SERIAL_COUNTER.next_serial(),
				time: 0,
			},
		);
		touch.frame(&mut state.lock());
	}
	pub fn touch_move(&self, id: u32, position: Vector2<f32>) {
		let Some(state) = self.wayland_state.upgrade() else {
			return;
		};
		let Some(surface) = self.touches.lock().get(&id).and_then(|c| c.upgrade().ok()) else {
			return;
		};
		let Some(touch) = self.seat.get_touch() else {
			return;
		};
		touch.motion(
			&mut state.lock(),
			Some((surface, (0, 0).into())),
			&touch::MotionEvent {
				slot: Some(id).into(),
				location: (position.x as f64, position.y as f64).into(),
				time: 0,
			},
		);
		touch.frame(&mut state.lock());
	}
	pub fn touch_up(&self, id: u32) {
		let Some(state) = self.wayland_state.upgrade() else {
			return;
		};
		let Some(touch) = self.seat.get_touch() else {
			return;
		};
		touch.up(
			&mut state.lock(),
			&UpEvent {
				slot: Some(id).into(),
				serial: SERIAL_COUNTER.next_serial(),
				time: 0,
			},
		);
	}
	pub fn reset_input(&self) {
		for id in self.touches.lock().keys() {
			self.touch_up(*id)
		}
	}
}

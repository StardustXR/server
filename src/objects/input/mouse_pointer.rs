use crate::{
	core::client::INTERNAL_CLIENT,
	nodes::{
		data::{
			mask_matches, pulse_receiver_client, PulseSender, KEYMAPS, PULSE_RECEIVER_REGISTRY,
		},
		fields::{FieldTrait, Ray},
		input::{InputDataType, InputHandler, InputMethod, Pointer, INPUT_HANDLER_REGISTRY},
		spatial::Spatial,
		Node, OwnedNode,
	},
};
use color_eyre::eyre::Result;
use glam::{vec3, Mat4, Vec3};
use mint::Vector2;
use serde::{Deserialize, Serialize};
use slotmap::{DefaultKey, Key as SlotKey};
use stardust_xr::values::Datamap;
use std::sync::Arc;
use stereokit_rust::system::{Input, Key};
use xkbcommon::xkb::{Context, Keymap, FORMAT_TEXT_V1};

use super::{get_sorted_handlers, CaptureManager, DistanceCalculator};

#[derive(Debug, Deserialize, Serialize)]
struct MouseEvent {
	select: f32,
	middle: f32,
	context: f32,
	grab: f32,
	scroll_continuous: Vector2<f32>,
	scroll_discrete: Vector2<f32>,
	raw_input_events: Vec<u32>,
}
impl Default for MouseEvent {
	fn default() -> Self {
		MouseEvent {
			select: 0.0,
			middle: 0.0,
			context: 0.0,
			grab: 0.0,
			scroll_continuous: [0.0; 2].into(),
			scroll_discrete: [0.0; 2].into(),
			raw_input_events: vec![],
		}
	}
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct KeyboardEvent {
	pub keyboard: (),
	pub xkbv1: (),
	pub keymap_id: u64,
	pub keys: Vec<i32>,
}

#[allow(unused)]
pub struct MousePointer {
	node: OwnedNode,
	keymap: DefaultKey,
	spatial: Arc<Spatial>,
	pointer: Arc<InputMethod>,
	capture_manager: CaptureManager,
	mouse_datamap: MouseEvent,
	keyboard_datamap: KeyboardEvent,
	keyboard_sender: Arc<PulseSender>,
}
impl MousePointer {
	pub fn new() -> Result<Self> {
		let node = Node::generate(&INTERNAL_CLIENT, false).add_to_scenegraph_owned()?;
		let spatial = Spatial::add_to(&node.0, None, Mat4::IDENTITY, false);
		let pointer = InputMethod::add_to(
			&node.0,
			InputDataType::Pointer(Pointer::default()),
			Datamap::from_typed(MouseEvent::default())?,
		)?;

		let keymap = KEYMAPS.lock().insert(
			Keymap::new_from_names(&Context::new(0), "evdev", "", "", "", None, 0)
				.unwrap()
				.get_as_string(FORMAT_TEXT_V1),
		);

		let keyboard_sender = PulseSender::add_to(
			&node.0,
			Datamap::from_typed(KeyboardEvent::default()).unwrap(),
		)
		.unwrap();

		Ok(MousePointer {
			node,
			spatial,
			pointer,
			capture_manager: CaptureManager::default(),
			mouse_datamap: Default::default(),
			keyboard_datamap: KeyboardEvent {
				keyboard: (),
				xkbv1: (),
				keymap_id: keymap.data().as_ffi(),
				keys: vec![],
			},
			keymap,
			keyboard_sender,
		})
	}
	pub fn update(&mut self) {
		let mouse = Input::get_mouse();

		let ray = mouse.get_ray();
		self.spatial.set_local_transform(
			Mat4::look_to_rh(
				Vec3::from(ray.position),
				Vec3::from(ray.direction),
				vec3(0.0, 1.0, 0.0),
			)
			.inverse(),
		);
		{
			// Set pointer input datamap
			self.mouse_datamap = MouseEvent {
				select: Input::key(Key::MouseLeft).is_active() as u32 as f32,
				middle: Input::key(Key::MouseCenter).is_active() as u32 as f32,
				context: Input::key(Key::MouseRight).is_active() as u32 as f32,
				grab: Input::key(Key::MouseBack).is_active() as u32 as f32,
				scroll_continuous: [0.0, mouse.scroll_change / 120.0].into(),
				scroll_discrete: [0.0, mouse.scroll_change / 120.0].into(),
				raw_input_events: vec![],
			};
			*self.pointer.datamap.lock() = Datamap::from_typed(&self.mouse_datamap).unwrap();
		}
		self.target_pointer_input();
		self.send_keyboard_input();
	}
	fn target_pointer_input(&mut self) {
		let distance_calculator: DistanceCalculator = |space, data, field| {
			let result = field.ray_march(Ray {
				origin: vec3(0.0, 0.0, 0.0),
				direction: vec3(0.0, 0.0, -1.0),
				space: space.clone(),
			});
			let valid =
				result.deepest_point_distance > 0.0 && result.min_distance.is_sign_negative();
			valid.then_some(result.deepest_point_distance)
		};

		self.capture_manager.update_capture(&self.pointer);
		self.capture_manager
			.set_new_capture(&self.pointer, distance_calculator);
		self.capture_manager.apply_capture(&self.pointer);

		if self.capture_manager.capture.is_some() {
			return;
		}

		let sorted_handlers = get_sorted_handlers(&self.pointer, distance_calculator);
		self.pointer.set_handler_order(sorted_handlers.iter());
	}

	fn send_keyboard_input(&mut self) {
		let rx = PULSE_RECEIVER_REGISTRY
			.get_valid_contents()
			.into_iter()
			.filter(|rx| mask_matches(&rx.mask, &self.keyboard_sender.mask))
			.map(|rx| {
				let result = rx.field.ray_march(Ray {
					origin: vec3(0.0, 0.0, 0.0),
					direction: vec3(0.0, 0.0, -1.0),
					space: self.spatial.clone(),
				});
				(rx, result)
			})
			.filter(|(_rx, result)| {
				result.deepest_point_distance > 0.0 && result.min_distance < 0.05
			})
			.reduce(|(rx_a, result_a), (rx_b, result_b)| {
				if result_a.deepest_point_distance < result_b.deepest_point_distance {
					(rx_a, result_a)
				} else {
					(rx_b, result_b)
				}
			})
			.map(|(rx, _)| rx);

		if let Some(rx) = rx {
			let keys = (8_u32..254)
				.map(|i| unsafe { std::mem::transmute(i) })
				.filter_map(|k| Some((map_key(k)?, Input::key(k))))
				.filter_map(|(i, k)| {
					if k.is_just_active() {
						Some(i as i32)
					} else if k.is_just_inactive() {
						Some(-(i as i32))
					} else {
						None
					}
				})
				.collect();

			self.keyboard_datamap.keys = keys;
			if !self.keyboard_datamap.keys.is_empty() {
				pulse_receiver_client::data(
					&rx.node.upgrade().unwrap(),
					&self.node.0,
					&Datamap::from_typed(&self.keyboard_datamap).unwrap(),
				)
				.unwrap();
			}
		}
	}
}

fn map_key(key: Key) -> Option<u32> {
	match key {
		Key::Backspace => Some(input_event_codes::KEY_BACKSPACE!()),
		Key::Tab => Some(input_event_codes::KEY_TAB!()),
		Key::Return => Some(input_event_codes::KEY_ENTER!()),
		Key::Shift => Some(input_event_codes::KEY_LEFTSHIFT!()),
		Key::Ctrl => Some(input_event_codes::KEY_LEFTCTRL!()),
		Key::Alt => Some(input_event_codes::KEY_LEFTALT!()),
		Key::CapsLock => Some(input_event_codes::KEY_CAPSLOCK!()),
		Key::Esc => Some(input_event_codes::KEY_ESC!()),
		Key::Space => Some(input_event_codes::KEY_SPACE!()),
		Key::End => Some(input_event_codes::KEY_END!()),
		Key::Home => Some(input_event_codes::KEY_HOME!()),
		Key::Left => Some(input_event_codes::KEY_LEFT!()),
		Key::Right => Some(input_event_codes::KEY_RIGHT!()),
		Key::Up => Some(input_event_codes::KEY_UP!()),
		Key::Down => Some(input_event_codes::KEY_DOWN!()),
		Key::PageUp => Some(input_event_codes::KEY_PAGEUP!()),
		Key::PageDown => Some(input_event_codes::KEY_PAGEDOWN!()),
		Key::PrintScreen => Some(input_event_codes::KEY_PRINT!()),
		Key::KeyInsert => Some(input_event_codes::KEY_INSERT!()),
		Key::Del => Some(input_event_codes::KEY_DELETE!()),
		Key::Key0 => Some(input_event_codes::KEY_0!()),
		Key::Key1 => Some(input_event_codes::KEY_1!()),
		Key::Key2 => Some(input_event_codes::KEY_2!()),
		Key::Key3 => Some(input_event_codes::KEY_3!()),
		Key::Key4 => Some(input_event_codes::KEY_4!()),
		Key::Key5 => Some(input_event_codes::KEY_5!()),
		Key::Key6 => Some(input_event_codes::KEY_6!()),
		Key::Key7 => Some(input_event_codes::KEY_7!()),
		Key::Key8 => Some(input_event_codes::KEY_8!()),
		Key::Key9 => Some(input_event_codes::KEY_9!()),
		Key::A => Some(input_event_codes::KEY_A!()),
		Key::B => Some(input_event_codes::KEY_B!()),
		Key::C => Some(input_event_codes::KEY_C!()),
		Key::D => Some(input_event_codes::KEY_D!()),
		Key::E => Some(input_event_codes::KEY_E!()),
		Key::F => Some(input_event_codes::KEY_F!()),
		Key::G => Some(input_event_codes::KEY_G!()),
		Key::H => Some(input_event_codes::KEY_H!()),
		Key::I => Some(input_event_codes::KEY_I!()),
		Key::J => Some(input_event_codes::KEY_J!()),
		Key::K => Some(input_event_codes::KEY_K!()),
		Key::L => Some(input_event_codes::KEY_L!()),
		Key::M => Some(input_event_codes::KEY_M!()),
		Key::N => Some(input_event_codes::KEY_N!()),
		Key::O => Some(input_event_codes::KEY_O!()),
		Key::P => Some(input_event_codes::KEY_P!()),
		Key::Q => Some(input_event_codes::KEY_Q!()),
		Key::R => Some(input_event_codes::KEY_R!()),
		Key::S => Some(input_event_codes::KEY_S!()),
		Key::T => Some(input_event_codes::KEY_T!()),
		Key::U => Some(input_event_codes::KEY_U!()),
		Key::V => Some(input_event_codes::KEY_V!()),
		Key::W => Some(input_event_codes::KEY_W!()),
		Key::X => Some(input_event_codes::KEY_X!()),
		Key::Y => Some(input_event_codes::KEY_Y!()),
		Key::Z => Some(input_event_codes::KEY_Z!()),
		Key::Numpad0 => Some(input_event_codes::KEY_NUMERIC_0!()),
		Key::Numpad1 => Some(input_event_codes::KEY_NUMERIC_1!()),
		Key::Numpad2 => Some(input_event_codes::KEY_NUMERIC_2!()),
		Key::Numpad3 => Some(input_event_codes::KEY_NUMERIC_3!()),
		Key::Numpad4 => Some(input_event_codes::KEY_NUMERIC_4!()),
		Key::Numpad5 => Some(input_event_codes::KEY_NUMERIC_5!()),
		Key::Numpad6 => Some(input_event_codes::KEY_NUMERIC_6!()),
		Key::Numpad7 => Some(input_event_codes::KEY_NUMERIC_7!()),
		Key::Numpad8 => Some(input_event_codes::KEY_NUMERIC_8!()),
		Key::Numpad9 => Some(input_event_codes::KEY_NUMERIC_9!()),
		Key::F1 => Some(input_event_codes::KEY_F1!()),
		Key::F2 => Some(input_event_codes::KEY_F2!()),
		Key::F3 => Some(input_event_codes::KEY_F3!()),
		Key::F4 => Some(input_event_codes::KEY_F4!()),
		Key::F5 => Some(input_event_codes::KEY_F5!()),
		// Key::F6 => Some(input_event_codes::KEY_F6!()),
		// Key::F7 => Some(input_event_codes::KEY_F7!()),
		// Key::F8 => Some(input_event_codes::KEY_F8!()),
		Key::F9 => Some(input_event_codes::KEY_F9!()),
		Key::F10 => Some(input_event_codes::KEY_F10!()),
		Key::F11 => Some(input_event_codes::KEY_F11!()),
		Key::F12 => Some(input_event_codes::KEY_F12!()),
		Key::Comma => Some(input_event_codes::KEY_COMMA!()),
		Key::Period => Some(input_event_codes::KEY_DOT!()),
		Key::SlashFwd => Some(input_event_codes::KEY_SLASH!()),
		Key::SlashBack => Some(input_event_codes::KEY_BACKSLASH!()),
		Key::Semicolon => Some(input_event_codes::KEY_SEMICOLON!()),
		Key::Apostrophe => Some(input_event_codes::KEY_APOSTROPHE!()),
		Key::BracketOpen => Some(input_event_codes::KEY_LEFTBRACE!()),
		Key::BracketClose => Some(input_event_codes::KEY_RIGHTBRACE!()),
		Key::Minus => Some(input_event_codes::KEY_MINUS!()),
		Key::Equals => Some(input_event_codes::KEY_EQUAL!()),
		Key::Backtick => Some(input_event_codes::KEY_GRAVE!()),
		Key::LCmd => Some(input_event_codes::KEY_LEFTMETA!()),
		Key::RCmd => Some(input_event_codes::KEY_RIGHTMETA!()),
		Key::Multiply => Some(input_event_codes::KEY_NUMERIC_STAR!()),
		Key::Add => Some(input_event_codes::KEY_KPPLUS!()),
		Key::Subtract => Some(input_event_codes::KEY_MINUS!()),
		Key::Decimal => Some(input_event_codes::KEY_DOT!()),
		Key::Divide => Some(input_event_codes::KEY_SLASH!()),
		_ => None,
	}
}

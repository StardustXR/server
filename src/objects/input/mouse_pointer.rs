use crate::{
	core::client::INTERNAL_CLIENT,
	nodes::{
		data::{
			mask_matches, pulse_receiver_client, PulseSender, KEYMAPS, PULSE_RECEIVER_REGISTRY,
		},
		fields::{Field, Ray},
		input::{InputDataType, InputHandler, InputMethod, Pointer, INPUT_HANDLER_REGISTRY},
		spatial::Spatial,
		Node,
	},
};
use color_eyre::eyre::Result;
use glam::{vec3, Mat4, Vec3};
use mint::Vector2;
use nanoid::nanoid;
use serde::{Deserialize, Serialize};
use stardust_xr::values::Datamap;
use std::sync::Arc;
use stereokit_rust::system::{Input, Key};
use xkbcommon::xkb::{Context, Keymap, FORMAT_TEXT_V1};

#[derive(Deserialize, Serialize)]
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KeyboardEvent {
	pub keyboard: (),
	pub xkbv1: (),
	pub keymap_id: String,
	pub keys: Vec<i32>,
}
impl Default for KeyboardEvent {
	fn default() -> Self {
		Self {
			keyboard: (),
			xkbv1: (),
			keymap_id: "flatscreen".to_string(),
			keys: Default::default(),
		}
	}
}

pub struct MousePointer {
	node: Arc<Node>,
	spatial: Arc<Spatial>,
	pointer: Arc<InputMethod>,
	capture: Option<Arc<InputHandler>>,
	mouse_datamap: MouseEvent,
	keyboard_datamap: KeyboardEvent,
	keyboard_sender: Arc<PulseSender>,
}
impl MousePointer {
	pub fn new() -> Result<Self> {
		let node = Node::create_parent_name(&INTERNAL_CLIENT, "", &nanoid!(), false)
			.add_to_scenegraph()?;
		let spatial = Spatial::add_to(&node, None, Mat4::IDENTITY, false);
		let pointer = InputMethod::add_to(
			&node,
			InputDataType::Pointer(Pointer::default()),
			Datamap::from_typed(MouseEvent::default())?,
		)?;

		KEYMAPS.lock().insert(
			"flatscreen".to_string(),
			Keymap::new_from_names(&Context::new(0), "evdev", "", "", "", None, 0)
				.unwrap()
				.get_as_string(FORMAT_TEXT_V1),
		);

		let keyboard_sender = PulseSender::add_to(
			&node,
			Datamap::from_typed(KeyboardEvent::default()).unwrap(),
		)
		.unwrap();

		Ok(MousePointer {
			node,
			spatial,
			pointer,
			capture: None,
			mouse_datamap: Default::default(),
			keyboard_datamap: Default::default(),
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
			self.mouse_datamap.select = if Input::key(Key::MouseLeft).is_active() {
				1.0f32
			} else {
				0.0f32
			};
			self.mouse_datamap.middle = if Input::key(Key::MouseCenter).is_active() {
				1.0f32
			} else {
				0.0f32
			};
			self.mouse_datamap.context = if Input::key(Key::MouseRight).is_active() {
				1.0f32
			} else {
				0.0f32
			};
			self.mouse_datamap.grab = if Input::key(Key::MouseBack).is_active()
				|| Input::key(Key::MouseForward).is_active()
			{
				1.0f32
			} else {
				0.0f32
			};
			self.mouse_datamap.scroll_continuous = [0.0, mouse.scroll_change / 120.0].into();
			self.mouse_datamap.scroll_discrete = [0.0, mouse.scroll_change / 120.0].into();
			*self.pointer.datamap.lock() = Datamap::from_typed(&self.mouse_datamap).unwrap();
		}
		self.target_pointer_input();
		self.send_keyboard_input();
	}

	fn target_pointer_input(&mut self) {
		// remove the capture when it's removed from captures list
		if let Some(capture) = &self.capture {
			if !self
				.pointer
				.capture_requests
				.get_valid_contents()
				.contains(&capture)
			{
				self.capture.take();
			}
		}
		// add the capture that's the closest if we don't have one
		if self.capture.is_none() {
			if let Some(new_capture) = self
				.pointer
				.capture_requests
				.get_valid_contents()
				.into_iter()
				.map(|h| {
					(
						h.clone(),
						h.field.ray_march(Ray {
							origin: vec3(0.0, 0.0, 0.0),
							direction: vec3(0.0, 0.0, -1.0),
							space: self.spatial.clone(),
						}),
					)
				})
				.reduce(|(handlers_a, result_a), (handlers_b, result_b)| {
					if result_a.min_distance < result_b.min_distance {
						(handlers_a, result_a)
					} else {
						(handlers_b, result_b)
					}
				})
				.map(|(rx, _)| rx)
			{
				self.capture = Some(new_capture.clone());
			}
		}

		// make sure that if something is captured only send input to it
		self.pointer.captures.clear();
		if let Some(capture) = &self.capture {
			self.pointer.set_handler_order([capture].into_iter());
			self.pointer.captures.add_raw(capture);
			return;
		}

		// send input to all the input handlers that are the closest to the ray as possible
		self.pointer.set_handler_order(
			INPUT_HANDLER_REGISTRY
				.get_valid_contents()
				.into_iter()
				// filter out all the disabled handlers
				.filter(|handler| {
					let Some(node) = handler.node.upgrade() else {
						return false;
					};
					node.enabled()
				})
				// ray march to all the enabled handlers' fields
				.map(|handler| {
					let result = handler.field.ray_march(Ray {
						origin: vec3(0.0, 0.0, 0.0),
						direction: vec3(0.0, 0.0, -1.0),
						space: self.spatial.clone(),
					});
					(vec![handler], result)
				})
				// make sure the field isn't at the pointer origin and that it's being hit
				.filter(|(_, result)| {
					result.deepest_point_distance > 0.01 && result.min_distance < 0.0
				})
				// .inspect(|(_, result)| {
				// 	dbg!(result);
				// })
				// now collect all handlers that are same distance if they're the closest
				.reduce(|(mut handlers_a, result_a), (handlers_b, result_b)| {
					if (result_a.deepest_point_distance - result_b.deepest_point_distance).abs()
						< 0.001
					{
						// distance is basically the same
						handlers_a.extend(handlers_b);
						(handlers_a, result_a)
					} else if result_a.deepest_point_distance < result_b.deepest_point_distance {
						(handlers_a, result_a)
					} else {
						(handlers_b, result_b)
					}
				})
				.map(|(rx, _)| rx)
				.unwrap_or_default()
				.iter(),
		);
	}

	fn send_keyboard_input(&mut self) {
		let rx = PULSE_RECEIVER_REGISTRY
			.get_valid_contents()
			.into_iter()
			.filter(|rx| mask_matches(&rx.mask, &self.keyboard_sender.mask))
			.map(|rx| {
				let result = rx.field_node.get_aspect::<Field>().unwrap().ray_march(Ray {
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
					&self.node.uid,
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
		Key::F6 => Some(input_event_codes::KEY_F6!()),
		Key::F7 => Some(input_event_codes::KEY_F7!()),
		Key::F8 => Some(input_event_codes::KEY_F8!()),
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

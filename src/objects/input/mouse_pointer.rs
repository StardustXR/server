use crate::{
	core::{client::INTERNAL_CLIENT, typed_datamap::TypedDatamap},
	nodes::{
		data::{mask_matches, Mask, PulseSender, KEYMAPS, PULSE_RECEIVER_REGISTRY},
		fields::Ray,
		input::{pointer::Pointer, InputMethod, InputType},
		spatial::Spatial,
		Node,
	},
};
use color_eyre::eyre::Result;
use glam::{vec2, vec3, Mat4, Vec2, Vec3};
use nanoid::nanoid;
use serde::{Deserialize, Serialize};
use std::{convert::TryFrom, sync::Arc};
use stereokit::{ray_from_mouse, ButtonState, Key, StereoKitMultiThread};
use tracing::instrument;

#[derive(Default, Deserialize, Serialize)]
struct MouseEvent {
	select: f32,
	grab: f32,
	scroll: Vec2,
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
	mouse_datamap: TypedDatamap<MouseEvent>,
	keyboard_datamap: TypedDatamap<KeyboardEvent>,
	keyboard_sender: Arc<PulseSender>,
}
impl MousePointer {
	pub fn new() -> Result<Self> {
		let node = Node::create(&INTERNAL_CLIENT, "", &nanoid!(), false).add_to_scenegraph()?;
		let spatial = Spatial::add_to(&node, None, Mat4::IDENTITY, false).unwrap();
		let pointer =
			InputMethod::add_to(&node, InputType::Pointer(Pointer::default()), None).unwrap();

		KEYMAPS
			.lock()
			.insert("flatscreen".to_string(), include_str!("sk.kmp").to_string());

		let keyboard_sender =
			PulseSender::add_to(&node, Mask::from_struct::<KeyboardEvent>()).unwrap();

		Ok(MousePointer {
			node,
			spatial,
			pointer,
			mouse_datamap: Default::default(),
			keyboard_datamap: Default::default(),
			keyboard_sender,
		})
	}
	#[instrument(level = "debug", name = "Update Flatscreen Pointer Ray", skip_all)]
	pub fn update(&mut self, sk: &impl StereoKitMultiThread) {
		let mouse = sk.input_mouse();

		let ray = ray_from_mouse(mouse.pos).unwrap();
		self.spatial.set_local_transform(
			Mat4::look_to_rh(
				Vec3::from(ray.pos),
				Vec3::from(ray.dir),
				vec3(0.0, 1.0, 0.0),
			)
			.inverse(),
		);
		{
			// Set pointer input datamap
			self.mouse_datamap.select =
				if sk.input_key(Key::MouseLeft).contains(ButtonState::ACTIVE) {
					1.0f32
				} else {
					0.0f32
				};
			self.mouse_datamap.grab = if sk.input_key(Key::MouseRight).contains(ButtonState::ACTIVE)
			{
				1.0f32
			} else {
				0.0f32
			};
			self.mouse_datamap.scroll = vec2(0.0, mouse.scroll_change / 120.0);
			*self.pointer.datamap.lock() = self.mouse_datamap.to_datamap().ok();
		}
		self.send_keyboard_input(sk);
	}

	fn send_keyboard_input(&mut self, sk: &impl StereoKitMultiThread) {
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
				.filter_map(|i| Some((i, Key::try_from(i).ok()?)))
				.map(|(i, k)| (i - 8, sk.input_key(k)))
				.filter_map(|(i, k)| {
					if k.contains(ButtonState::JUST_ACTIVE) {
						Some(i as i32)
					} else if k.contains(ButtonState::JUST_INACTIVE) {
						Some(-(i as i32))
					} else {
						None
					}
				})
				.collect();

			self.keyboard_datamap.keys = keys;
			if !self.keyboard_datamap.keys.is_empty() {
				rx.send_data(&self.node.uid, self.keyboard_datamap.serialize().unwrap())
					.unwrap();
			}
		}
	}
}

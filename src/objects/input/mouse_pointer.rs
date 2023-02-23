use crate::{
	core::client::INTERNAL_CLIENT,
	nodes::{
		data::{mask_matches, Mask, PulseSender, PULSE_RECEIVER_REGISTRY},
		fields::Ray,
		input::{pointer::Pointer, InputMethod, InputType},
		spatial::Spatial,
		Node,
	},
};
use color_eyre::eyre::Result;
use glam::{vec3, Mat4, Vec3};
use nanoid::nanoid;
use serde::Serialize;
use stardust_xr::schemas::{flat::Datamap, flex::flexbuffers};
use std::{convert::TryFrom, sync::Arc};
use stereokit::input::{ButtonState, Key, Ray as SkRay, StereoKitInput};
use tracing::instrument;

const SK_KEYMAP: &str = include_str!("sk.kmp");

#[derive(Debug, Clone, Serialize)]
pub struct KeyboardEvent {
	pub keyboard: String,
	pub keymap: Option<String>,
	pub keys_up: Option<Vec<u32>>,
	pub keys_down: Option<Vec<u32>>,
}

pub struct MousePointer {
	node: Arc<Node>,
	spatial: Arc<Spatial>,
	pointer: Arc<InputMethod>,
	keyboard_sender: Arc<PulseSender>,
}
impl MousePointer {
	pub fn new() -> Result<Self> {
		let node = Node::create(&INTERNAL_CLIENT, "", &nanoid!(), false).add_to_scenegraph()?;
		let spatial = Spatial::add_to(&node, None, Mat4::IDENTITY, false).unwrap();
		let pointer =
			InputMethod::add_to(&node, InputType::Pointer(Pointer::default()), None).unwrap();

		let keyboard_mask = {
			let mut fbb = flexbuffers::Builder::default();
			let mut map = fbb.start_map();
			map.push("keyboard", "xkbv1");
			map.end_map();
			Mask(fbb.take_buffer())
		};
		let keyboard_sender = PulseSender::add_to(&node, keyboard_mask).unwrap();

		Ok(MousePointer {
			node,
			spatial,
			pointer,
			keyboard_sender,
		})
	}
	#[instrument(level = "debug", name = "Update Flatscreen Pointer Ray", skip_all)]
	pub fn update(&self, sk: &impl StereoKitInput) {
		let mouse = sk.input_mouse();

		if let Some(ray) = SkRay::from_mouse(&mouse) {
			self.spatial.set_local_transform(
				Mat4::look_to_rh(ray.pos.into(), -Vec3::from(ray.dir), vec3(0.0, 1.0, 0.0))
					.inverse(),
			)
		}
		{
			// Set pointer input datamap
			let mut fbb = flexbuffers::Builder::default();
			let mut map = fbb.start_map();
			map.push(
				"select",
				if sk.input_key(Key::MouseLeft).contains(ButtonState::Active) {
					1.0f32
				} else {
					0.0f32
				},
			);
			map.push(
				"grab",
				if sk.input_key(Key::MouseRight).contains(ButtonState::Active) {
					1.0f32
				} else {
					0.0f32
				},
			);
			let mut scroll_vec = map.start_vector("scroll");
			scroll_vec.push(0_f32);
			scroll_vec.push(mouse.scroll_change / 120.0);
			scroll_vec.end_vector();
			map.end_map();
			*self.pointer.datamap.lock() = Datamap::new(fbb.take_buffer()).ok();
		}
		self.send_keyboard_input(sk);
	}

	fn send_keyboard_input(&self, sk: &impl StereoKitInput) {
		let rx = PULSE_RECEIVER_REGISTRY
			.get_valid_contents()
			.into_iter()
			.filter(|rx| mask_matches(&rx.mask, &self.keyboard_sender.mask))
			.map(|rx| {
				let result = rx.field.ray_march(Ray {
					origin: vec3(0.0, 0.0, 0.0),
					direction: vec3(0.0, 0.0, 1.0),
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
			let mut keys_up = vec![];
			let mut keys_down = vec![];
			let keys = (8_u32..254)
				.filter_map(|i| Some((i, Key::try_from(i).ok()?)))
				.map(|(i, k)| (i - 8, sk.input_key(k)));
			for (key, state) in keys {
				if state.contains(ButtonState::JustActive) {
					keys_down.push(key);
				} else if state.contains(ButtonState::JustInactive) {
					keys_up.push(key);
				}
			}

			let key_event = KeyboardEvent {
				keyboard: "xkbv1".to_string(),
				keymap: Some(SK_KEYMAP.to_string()),
				keys_up: Some(keys_up),
				keys_down: Some(keys_down),
			};
			let mut serializer = flexbuffers::FlexbufferSerializer::new();
			let _ = key_event.serialize(&mut serializer);
			rx.send_data(&self.node.uid, serializer.take_buffer())
				.unwrap();
		}
	}
}

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
use glam::{vec3, Mat4};
use nanoid::nanoid;
use stardust_xr::{schemas::flat::Datamap, values::Transform};
use std::{convert::TryFrom, sync::Arc};
use stereokit::{
	input::{ButtonState, Key, Ray as SkRay},
	StereoKit,
};

const SK_KEYMAP: &'static str = include_str!("sk.kmp");

pub struct MousePointer {
	node: Arc<Node>,
	spatial: Arc<Spatial>,
	pointer: Arc<InputMethod>,
	keyboard_sender: Arc<PulseSender>,
}
impl MousePointer {
	pub fn new() -> Self {
		let node = Node::create(&INTERNAL_CLIENT, "", &nanoid!(), false).add_to_scenegraph();
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

		MousePointer {
			node,
			spatial,
			pointer,
			keyboard_sender,
		}
	}
	pub fn update(&self, sk: &StereoKit) {
		let mouse = sk.input_mouse();

		if let Some(ray) = SkRay::from_mouse(mouse) {
			self.spatial.set_local_transform_components(
				None,
				Transform {
					position: Some(ray.pos),
					rotation: Some(
						glam::Quat::from_rotation_arc(vec3(0.0, 0.0, 1.0), ray.dir.into()).into(),
					),
					scale: None,
				},
			);
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

	fn send_keyboard_input(&self, sk: &StereoKit) {
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
			for (key, state) in (1_u32..254)
				.filter_map(|i| Some((i, Key::try_from(i).ok()?)))
				.map(|(i, k)| (i, sk.input_key(k)))
				.filter(|(_, k)| k.contains(ButtonState::Changed))
			{
				if state.contains(ButtonState::Active) {
					keys_down.push(key);
				} else {
					keys_up.push(key);
				}
			}

			let mut fbb = flexbuffers::Builder::default();
			{
				let mut map = fbb.start_map();
				map.push("keyboard", "xkbv1");
				map.push("keymap", SK_KEYMAP);
				{
					let mut keys_up_flex = map.start_vector("keys_up");
					for key in keys_up {
						keys_up_flex.push(key);
					}
				}
				{
					let mut keys_down_flex = map.start_vector("keys_down");
					for key in keys_down {
						keys_down_flex.push(key);
					}
				}
			}
			rx.send_data(&self.node.uid, fbb.take_buffer()).unwrap();
		}
	}
}

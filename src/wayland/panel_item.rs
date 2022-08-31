use crate::{
	core::{
		client::{Client, INTERNAL_CLIENT},
		registry::Registry,
	},
	nodes::{
		core::Node,
		item::{register_item_ui_flex, Item, ItemSpecialization, ItemType, TypeInfo},
		spatial::Spatial,
	},
};
use anyhow::{anyhow, Result};
use glam::Mat4;
use lazy_static::lazy_static;
use libstardustxr::{flex::flexbuffer_from_vector_arguments, flex_to_vec2};
use nanoid::nanoid;
use parking_lot::Mutex;
use smithay::{
	backend::renderer::utils::RendererSurfaceStateUserData,
	reexports::wayland_server::{
		protocol::{
			wl_keyboard::KeyState,
			wl_pointer::{Axis, ButtonState},
			wl_surface::WlSurface,
		},
		DisplayHandle, Resource,
	},
	utils::{user_data::UserDataMap, Logical, Size},
	wayland::compositor,
};
use std::{
	convert::TryInto,
	sync::{Arc, Weak},
};

use super::{seat::SeatData, surface::CoreSurface, WaylandState};

lazy_static! {
	static ref ITEM_TYPE_INFO_PANEL: TypeInfo = TypeInfo {
		type_name: "panel",
		aliased_local_signals: vec![
			"applySurfaceMaterial",
			"setPointerActive",
			"setPointerPosition",
			"setPointerButtonPressed",
			"scrollPointerAxis",
			"touchDown",
			"touchMove",
			"touchUp",
			"setKeyboardActive",
			"setKeymap",
			"setKeyState",
			"setKeyModStates",
			"setKeyRepeat",
			"resize",
			"close",
		],
		aliased_local_methods: vec![],
		aliased_remote_signals: vec![],
		aliased_remote_methods: vec![],
		ui: Default::default(),
		items: Registry::new(),
		acceptors: Registry::new(),
	};
}

pub struct PanelItem {
	node: Weak<Node>,
	pub toplevel_surface: WlSurface,
	seat_data: SeatData,
	size: Mutex<Size<i32, Logical>>,
}
impl PanelItem {
	pub fn create(
		dh: &DisplayHandle,
		data: &UserDataMap,
		toplevel_surface: WlSurface,
	) -> Arc<Node> {
		let node = Node::create(&INTERNAL_CLIENT, "/item/panel/item", &nanoid!(), true)
			.add_to_scenegraph();
		Spatial::add_to(&node, None, Mat4::IDENTITY).unwrap();

		let seat_data = SeatData::new(toplevel_surface.client_id().unwrap());
		dh.create_global::<WaylandState, _, _>(7, seat_data.clone());

		let size = data
			.get::<RendererSurfaceStateUserData>()
			.unwrap()
			.borrow()
			.surface_size()
			.map(Mutex::new)
			.unwrap();

		let specialization = ItemType::Panel(PanelItem {
			node: Arc::downgrade(&node),
			toplevel_surface,
			seat_data,
			size,
		});
		let item =
			ITEM_TYPE_INFO_PANEL
				.items
				.add(Item::new(&node, &ITEM_TYPE_INFO_PANEL, specialization));
		let _ = node.item.set(item);
		node.add_local_signal("applySurfaceMaterial", PanelItem::apply_surface_material);
		node.add_local_signal("pointerDeactivate", PanelItem::pointer_deactivate);
		node.add_local_signal("pointerScroll", PanelItem::pointer_scroll);
		node.add_local_signal("pointerButton", PanelItem::pointer_button);
		node.add_local_signal("pointerMotion", PanelItem::pointer_motion);
		node.add_local_signal("keyboardSetActive", PanelItem::keyboard_set_active);
		node.add_local_signal("keyboardSetKeyState", PanelItem::keyboard_set_key_state);
		node.add_local_signal("keyboardSetModifiers", PanelItem::keyboard_set_modifiers);
		node
	}

	pub fn resize(&self, data: &UserDataMap) {
		if let Some(surface_states) = data.get::<RendererSurfaceStateUserData>() {
			if let Some(size) = surface_states.borrow().buffer_size() {
				let _ = self.node.upgrade().unwrap().send_remote_signal(
					"resize",
					&flexbuffer_from_vector_arguments(|vec| {
						vec.push(size.w as u64);
						vec.push(size.h as u64);
					}),
				);
			}
		}
	}

	fn apply_surface_material(node: &Node, calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		let flex_vec = flexbuffers::Reader::get_root(data)?.get_vector()?;
		let material_idx = flex_vec.idx(1).get_u64()?;
		let model_node = calling_client
			.scenegraph
			.get_node(flex_vec.idx(0).as_str())
			.ok_or_else(|| anyhow!("Model node not found"))?;
		let model = model_node
			.model
			.get()
			.ok_or_else(|| anyhow!("Node is not a model"))?;

		if let ItemType::Panel(panel_item) = &node.item.get().unwrap().specialization {
			compositor::with_states(&panel_item.toplevel_surface, |states| {
				let sk_mat = states
					.data_map
					.get::<CoreSurface>()
					.unwrap()
					.sk_mat
					.get()
					.unwrap()
					.clone();
				model
					.pending_material_replacements
					.lock()
					.insert(material_idx as u32, sk_mat);
			});
		}

		Ok(())
	}

	fn pointer_deactivate(node: &Node, _calling_client: Arc<Client>, _data: &[u8]) -> Result<()> {
		if let ItemType::Panel(panel_item) = &node.item.get().unwrap().specialization {
			if *panel_item.seat_data.pointer_active.lock() {
				if let Some(pointer) = panel_item.seat_data.pointer() {
					pointer.leave(0, &panel_item.toplevel_surface);
					*panel_item.seat_data.pointer_active.lock() = false;
				}
			}
		}

		Ok(())
	}

	fn pointer_motion(node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		if let ItemType::Panel(panel_item) = &node.item.get().unwrap().specialization {
			if let Some(pointer) = panel_item.seat_data.pointer() {
				let flex_vec = flexbuffers::Reader::get_root(data)?.get_vector()?;
				let x = flex_vec.index(0)?.get_f64()?;
				let y = flex_vec.index(1)?.get_f64()?;
				let mut pointer_active = panel_item.seat_data.pointer_active.lock();
				if *pointer_active {
					pointer.motion(0, x, y);
				} else {
					pointer.enter(0, &panel_item.toplevel_surface, x, y);
					*pointer_active = true;
				}
				pointer.frame();
			}
		}

		Ok(())
	}

	fn pointer_button(node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		if let ItemType::Panel(panel_item) = &node.item.get().unwrap().specialization {
			if let Some(pointer) = panel_item.seat_data.pointer() {
				if *panel_item.seat_data.pointer_active.lock() {
					let flex_vec = flexbuffers::Reader::get_root(data)?.get_vector()?;
					let button = flex_vec.index(0)?.get_u64()? as u32;
					let active = flex_vec.index(1)?.get_bool()?;
					pointer.button(
						0,
						0,
						button,
						if active {
							ButtonState::Pressed
						} else {
							ButtonState::Released
						},
					);
					pointer.frame();
				}
			}
		}

		Ok(())
	}

	fn pointer_scroll(node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		if let ItemType::Panel(panel_item) = &node.item.get().unwrap().specialization {
			if let Some(pointer) = panel_item.seat_data.pointer() {
				if *panel_item.seat_data.pointer_active.lock() {
					let flex = flexbuffers::Reader::get_root(data)?;
					if flex.flexbuffer_type().is_null() {
						pointer.axis_stop(0, Axis::HorizontalScroll);
						pointer.axis_stop(0, Axis::VerticalScroll);
					} else {
						let flex_vec = flex.get_vector()?;
						let axis_continuous_vec = flex_to_vec2!(flex_vec.idx(0))
							.ok_or_else(|| anyhow!("No continuous axis vector!"))?;
						pointer.axis(0, Axis::HorizontalScroll, axis_continuous_vec.x as f64);
						pointer.axis(0, Axis::VerticalScroll, axis_continuous_vec.y as f64);
						if let Some(axis_discrete_vec) = flex_to_vec2!(flex_vec.idx(0)) {
							pointer
								.axis_discrete(Axis::HorizontalScroll, axis_discrete_vec.x as i32);
							pointer.axis_discrete(Axis::VerticalScroll, axis_discrete_vec.y as i32);
						}
					}
					pointer.frame();
				}
			}
		}

		Ok(())
	}

	fn keyboard_set_active(node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		if let ItemType::Panel(panel_item) = &node.item.get().unwrap().specialization {
			if let Some(keyboard) = panel_item.seat_data.keyboard() {
				let mut keyboard_active = panel_item.seat_data.keyboard_active.lock();
				let active = flexbuffers::Reader::get_root(data)?.get_bool()?;
				if *keyboard_active != active {
					if active {
						keyboard.enter(0, &panel_item.toplevel_surface, vec![]);
					} else {
						keyboard.leave(0, &panel_item.toplevel_surface);
					}
					*keyboard_active = active;
				}
			}
		}

		Ok(())
	}

	fn keyboard_set_key_state(
		node: &Node,
		_calling_client: Arc<Client>,
		data: &[u8],
	) -> Result<()> {
		if let ItemType::Panel(panel_item) = &node.item.get().unwrap().specialization {
			if let Some(keyboard) = panel_item.seat_data.keyboard() {
				let active = *panel_item.seat_data.keyboard_active.lock();
				if active {
					let flex_vec = flexbuffers::Reader::get_root(data)?.get_vector()?;
					let key = flex_vec.index(0)?.get_u64()? as u32;
					let state: KeyState = (flex_vec.index(1)?.as_u64() as u32)
						.try_into()
						.map_err(|_| anyhow!("Invalid key state"))?;
					keyboard.key(0, 0, key, state);
				}
			}
		}

		Ok(())
	}

	fn keyboard_set_modifiers(
		node: &Node,
		_calling_client: Arc<Client>,
		data: &[u8],
	) -> Result<()> {
		if let ItemType::Panel(panel_item) = &node.item.get().unwrap().specialization {
			if let Some(keyboard) = panel_item.seat_data.keyboard() {
				let active = *panel_item.seat_data.keyboard_active.lock();
				if active {
					let flex_vec = flexbuffers::Reader::get_root(data)?.get_vector()?;
					keyboard.modifiers(
						0,
						flex_vec.index(0)?.get_u64()? as u32,
						flex_vec.index(1)?.get_u64()? as u32,
						flex_vec.index(2)?.get_u64()? as u32,
						flex_vec.index(3)?.get_u64()? as u32,
					);
				}
			}
		}

		Ok(())
	}
}
impl ItemSpecialization for PanelItem {
	fn serialize_start_data(&self, vec: &mut flexbuffers::VectorBuilder) {
		let mut size_vec = vec.start_vector();
		let size = *self.size.lock();
		size_vec.push(size.w as u32);
		size_vec.push(size.h as u32);
	}
}

pub fn register_panel_item_ui_flex(
	_node: &Node,
	calling_client: Arc<Client>,
	_data: &[u8],
) -> Result<()> {
	register_item_ui_flex(calling_client, &ITEM_TYPE_INFO_PANEL)
}

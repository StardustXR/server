use crate::{
	core::{
		client::{Client, INTERNAL_CLIENT},
		registry::Registry,
	},
	nodes::{
		core::Node,
		item::{register_item_ui_flex, Item, ItemSpecialization, ItemType, TypeInfo},
		model::Model,
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
		backend::ObjectId,
		protocol::{
			wl_keyboard::KeyState,
			wl_pointer::{Axis, ButtonState},
			wl_surface::WlSurface,
		},
		DisplayHandle, Resource,
	},
	utils::{user_data::UserDataMap, Logical, Size},
};
use std::{
	convert::TryInto,
	sync::{Arc, Weak},
};

use super::{seat::SeatData, surface::CoreSurface, WaylandState, GLOBAL_DESTROY_QUEUE_IN};

lazy_static! {
	static ref ITEM_TYPE_INFO_PANEL: TypeInfo = TypeInfo {
		type_name: "panel",
		aliased_local_signals: vec![
			"applySurfaceMaterial",
			"pointerDeactivate",
			"pointerScroll",
			"pointerButton",
			"pointerMotion",
			"keyboardSetActive",
			"keyboardSetKeyState",
			"keyboardSetModifiers",
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
	pending_material_applications: Mutex<Vec<(Arc<Model>, u32)>>,
	dh: DisplayHandle,
	pub toplevel_surface_id: ObjectId,
	seat_data: SeatData,
	size: Mutex<Size<i32, Logical>>,
}
impl PanelItem {
	pub fn create(
		dh: &DisplayHandle,
		data: &UserDataMap,
		toplevel_surface: WlSurface,
	) -> Arc<Node> {
		let node = Arc::new(Node::create(
			&INTERNAL_CLIENT,
			"/item/panel/item",
			&nanoid!(),
			true,
		));
		Spatial::add_to(&node, None, Mat4::IDENTITY).unwrap();

		let seat_data = SeatData::new(dh, toplevel_surface.client_id().unwrap());

		let size = data
			.get::<RendererSurfaceStateUserData>()
			.unwrap()
			.borrow()
			.surface_size()
			.map(Mutex::new)
			.unwrap();

		let specialization = ItemType::Panel(PanelItem {
			node: Arc::downgrade(&node),
			pending_material_applications: Mutex::new(Vec::new()),
			dh: dh.clone(),
			toplevel_surface_id: toplevel_surface.id(),
			seat_data,
			size,
		});
		Item::add_to(&node, &ITEM_TYPE_INFO_PANEL, specialization);
		node.add_local_signal(
			"applySurfaceMaterial",
			PanelItem::apply_surface_material_flex,
		);
		node.add_local_signal("pointerDeactivate", PanelItem::pointer_deactivate_flex);
		node.add_local_signal("pointerScroll", PanelItem::pointer_scroll_flex);
		node.add_local_signal("pointerButton", PanelItem::pointer_button_flex);
		node.add_local_signal("pointerMotion", PanelItem::pointer_motion_flex);
		node.add_local_signal("keyboardSetActive", PanelItem::keyboard_set_active_flex);
		node.add_local_signal(
			"keyboardSetKeyState",
			PanelItem::keyboard_set_key_state_flex,
		);
		node.add_local_signal(
			"keyboardSetModifiers",
			PanelItem::keyboard_set_modifiers_flex,
		);
		node
	}

	fn toplevel_surface(&self) -> WlSurface {
		WlSurface::from_id(&self.dh, self.toplevel_surface_id.clone()).unwrap()
	}

	pub fn resize(&self, data: &UserDataMap) {
		if let Some(surface_states) = data.get::<RendererSurfaceStateUserData>() {
			if let Some(size) = surface_states.borrow().surface_size() {
				*self.size.lock() = size;
				let _ = self.node.upgrade().unwrap().send_remote_signal(
					"resize",
					&flexbuffer_from_vector_arguments(|vec| {
						vec.push(size.w);
						vec.push(size.h);
					}),
				);
			}
		}
	}

	fn apply_surface_material_flex(
		node: &Node,
		calling_client: Arc<Client>,
		data: &[u8],
	) -> Result<()> {
		let flex_vec = flexbuffers::Reader::get_root(data)?.get_vector()?;
		let model_node = calling_client
			.scenegraph
			.get_node(flex_vec.idx(0).as_str())
			.ok_or_else(|| anyhow!("Model node not found"))?;
		let model = model_node
			.model
			.get()
			.ok_or_else(|| anyhow!("Node is not a model"))?;
		let material_idx = flex_vec.idx(1).get_u64()?;

		if let ItemType::Panel(panel_item) = &node.item.get().unwrap().specialization {
			panel_item
				.pending_material_applications
				.lock()
				.push((model.clone(), material_idx as u32));
		}

		Ok(())
	}

	pub fn apply_surface_materials(node: &Node, core_surface: &CoreSurface) {
		if let ItemType::Panel(panel_item) = &node.item.get().unwrap().specialization {
			let mut pending_material_applications = panel_item.pending_material_applications.lock();
			for (model, material_idx) in &*pending_material_applications {
				let sk_mat = core_surface.sk_mat.get().unwrap().clone();
				model
					.pending_material_replacements
					.lock()
					.insert(*material_idx, sk_mat);
			}
			pending_material_applications.clear();
		}
	}

	fn pointer_deactivate_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		_data: &[u8],
	) -> Result<()> {
		if let ItemType::Panel(panel_item) = &node.item.get().unwrap().specialization {
			if *panel_item.seat_data.pointer_active.lock() {
				if let Some(pointer) = panel_item.seat_data.pointer() {
					pointer.leave(0, &panel_item.toplevel_surface());
					*panel_item.seat_data.pointer_active.lock() = false;
				}
			}
		}

		Ok(())
	}

	fn pointer_motion_flex(node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		if let ItemType::Panel(panel_item) = &node.item.get().unwrap().specialization {
			if let Some(pointer) = panel_item.seat_data.pointer() {
				let flex_vec = flexbuffers::Reader::get_root(data)?.get_vector()?;
				let x = flex_vec.index(0)?.get_f64()?;
				let y = flex_vec.index(1)?.get_f64()?;
				let mut pointer_active = panel_item.seat_data.pointer_active.lock();
				if *pointer_active {
					pointer.motion(0, x, y);
				} else {
					pointer.enter(0, &panel_item.toplevel_surface(), x, y);
					*pointer_active = true;
				}
				pointer.frame();
			}
		}

		Ok(())
	}

	fn pointer_button_flex(node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
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

	fn pointer_scroll_flex(node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
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

	fn keyboard_set_active_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		data: &[u8],
	) -> Result<()> {
		if let ItemType::Panel(panel_item) = &node.item.get().unwrap().specialization {
			if let Some(keyboard) = panel_item.seat_data.keyboard() {
				let mut keyboard_active = panel_item.seat_data.keyboard_active.lock();
				let active = flexbuffers::Reader::get_root(data)?.get_bool()?;
				if *keyboard_active != active {
					if active {
						keyboard.enter(0, &panel_item.toplevel_surface(), vec![]);
					} else {
						keyboard.leave(0, &panel_item.toplevel_surface());
					}
					*keyboard_active = active;
				}
			}
		}

		Ok(())
	}

	fn keyboard_set_key_state_flex(
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

	fn keyboard_set_modifiers_flex(
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
impl Drop for PanelItem {
	fn drop(&mut self) {
		GLOBAL_DESTROY_QUEUE_IN
			.get()
			.unwrap()
			.send(self.seat_data.global_id.get().cloned().unwrap())
			.unwrap();
	}
}

pub fn register_panel_item_ui_flex(
	_node: &Node,
	calling_client: Arc<Client>,
	_data: &[u8],
) -> Result<()> {
	register_item_ui_flex(calling_client, &ITEM_TYPE_INFO_PANEL)
}

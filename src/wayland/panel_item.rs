use super::{seat::SeatData, surface::CoreSurface};
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
use anyhow::{anyhow, bail, Result};
use glam::Mat4;
use lazy_static::lazy_static;
use libstardustxr::{
	flex::{flexbuffer_from_arguments, flexbuffer_from_vector_arguments},
	flex_to_vec2,
};
use nanoid::nanoid;
use smithay::{
	reexports::wayland_server::{
		protocol::{
			wl_keyboard::KeyState,
			wl_pointer::{Axis, ButtonState},
		},
		Resource,
	},
	wayland::{compositor::SurfaceData, shell::xdg::XdgToplevelSurfaceData},
};
use std::{
	convert::TryInto,
	sync::{Arc, Weak},
};

lazy_static! {
	static ref ITEM_TYPE_INFO_PANEL: TypeInfo = TypeInfo {
		type_name: "panel",
		aliased_local_signals: vec![
			"applySurfaceMaterial",
			"applyCursorMaterial",
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
		aliased_remote_signals: vec!["resize", "setCursor",],
		aliased_remote_methods: vec![],
		ui: Default::default(),
		items: Registry::new(),
		acceptors: Registry::new(),
	};
}

pub struct PanelItem {
	node: Weak<Node>,
	core_surface: Weak<CoreSurface>,
	seat_data: SeatData,
}
impl PanelItem {
	pub fn create(core_surface: &Arc<CoreSurface>) -> Arc<Node> {
		let node = Arc::new(Node::create(
			&INTERNAL_CLIENT,
			"/item/panel/item",
			&nanoid!(),
			true,
		));
		Spatial::add_to(&node, None, Mat4::IDENTITY).unwrap();

		let seat_data = SeatData::new(
			&core_surface.dh,
			core_surface.wl_surface().client_id().unwrap(),
		);

		let specialization = ItemType::Panel(PanelItem {
			node: Arc::downgrade(&node),
			core_surface: Arc::downgrade(core_surface),
			seat_data,
		});
		let item = Item::add_to(&node, &ITEM_TYPE_INFO_PANEL, specialization);
		if let ItemType::Panel(panel) = &item.specialization {
			let _ = panel.seat_data.panel_item.set(Arc::downgrade(&item));
		}

		node.add_local_signal(
			"applySurfaceMaterial",
			PanelItem::apply_surface_material_flex,
		);
		node.add_local_signal("applyCursorMaterial", PanelItem::apply_cursor_material_flex);
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

	fn from_node(node: &Node) -> &PanelItem {
		match &node.item.get().unwrap().specialization {
			ItemType::Panel(panel_item) => panel_item,
			_ => unreachable!(),
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
		let material_idx = flex_vec.idx(1).get_u64()? as u32;

		if let ItemType::Panel(panel_item) = &node.item.get().unwrap().specialization {
			if let Some(core_surface) = panel_item.core_surface.upgrade() {
				core_surface.apply_material(model.clone(), material_idx);
			}
		}

		Ok(())
	}

	fn apply_cursor_material_flex(
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
		let material_idx = flex_vec.idx(1).get_u64()? as u32;

		if let ItemType::Panel(panel_item) = &node.item.get().unwrap().specialization {
			if let Some(cursor) = &*panel_item.seat_data.cursor.lock() {
				if let Some(core_surface) = cursor.lock().core_surface.upgrade() {
					core_surface.apply_material(model.clone(), material_idx);
				}
			}
		}

		Ok(())
	}

	pub fn on_mapped(core_surface: &Arc<CoreSurface>, surface_data: &SurfaceData) {
		if surface_data
			.data_map
			.get::<XdgToplevelSurfaceData>()
			.is_some()
		{
			surface_data
				.data_map
				.insert_if_missing_threadsafe(|| PanelItem::create(core_surface));
		}
	}

	pub fn if_mapped(core_surface: &Arc<CoreSurface>, surface_data: &SurfaceData) {
		if let Some(panel_node) = surface_data.data_map.get::<Arc<Node>>() {
			let panel_item = PanelItem::from_node(panel_node);

			core_surface.with_data(|core_surface_data| {
				if core_surface_data.resized {
					panel_item.resize(core_surface);
					core_surface_data.resized = false;
				}
			});

			panel_item.set_cursor();
		}
	}

	pub fn resize(&self, core_surface: &CoreSurface) {
		core_surface.with_data(|data| {
			if data.resized {
				let _ = self.node.upgrade().unwrap().send_remote_signal(
					"resize",
					&flexbuffer_from_vector_arguments(|vec| {
						vec.push(data.size.x);
						vec.push(data.size.y);
					}),
				);
				data.resized = false;
			}
		});
	}

	pub fn set_cursor(&self) {
		let mut cursor_changed = self.seat_data.cursor_changed.lock();
		if !*cursor_changed {
			return;
		}
		let mut data = flexbuffer_from_arguments(|flex| {
			flex.build_singleton(());
		});

		if let Some(cursor) = &*self.seat_data.cursor.lock() {
			let cursor = cursor.lock();
			if let Some(core_surface) = cursor.core_surface.upgrade() {
				if let Some(mapped_data) = &*core_surface.mapped_data.lock() {
					data = flexbuffer_from_vector_arguments(|vec| {
						let mut size_vec = vec.start_vector();
						let size = mapped_data.size;
						size_vec.push(size.x);
						size_vec.push(size.y);
						size_vec.end_vector();

						let mut hotspot_vec = vec.start_vector();
						hotspot_vec.push(cursor.hotspot.x);
						hotspot_vec.push(cursor.hotspot.y);
						hotspot_vec.end_vector();
					});
				} else {
					return;
				};
			} else {
				return;
			}
		}

		let _ = self
			.node
			.upgrade()
			.unwrap()
			.send_remote_signal("setCursor", &data);
		*cursor_changed = false;
	}

	fn pointer_deactivate_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		_data: &[u8],
	) -> Result<()> {
		let panel_item = PanelItem::from_node(node);
		if *panel_item.seat_data.pointer_active.lock() {
			if let Some(core_surface) = panel_item.core_surface.upgrade() {
				if let Some(pointer) = panel_item.seat_data.pointer() {
					pointer.leave(0, &core_surface.wl_surface());
					*panel_item.seat_data.pointer_active.lock() = false;
					pointer.frame();
					core_surface.flush_clients();
				}
			}
		}

		Ok(())
	}

	fn pointer_motion_flex(node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		if let ItemType::Panel(panel_item) = &node.item.get().unwrap().specialization {
			if let Some(pointer) = panel_item.seat_data.pointer() {
				if let Some(core_surface) = panel_item.core_surface.upgrade() {
					let flex_vec = flexbuffers::Reader::get_root(data)?.get_vector()?;
					let x = flex_vec.index(0)?.get_f64()?;
					let y = flex_vec.index(1)?.get_f64()?;
					let mut pointer_active = panel_item.seat_data.pointer_active.lock();
					if *pointer_active {
						pointer.motion(0, x, y);
					} else {
						pointer.enter(0, &core_surface.wl_surface(), x, y);
						*pointer_active = true;
					}
					pointer.frame();
					core_surface.flush_clients();
				}
			}
		}

		Ok(())
	}

	fn pointer_button_flex(node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		if let ItemType::Panel(panel_item) = &node.item.get().unwrap().specialization {
			if let Some(pointer) = panel_item.seat_data.pointer() {
				if *panel_item.seat_data.pointer_active.lock() {
					if let Some(core_surface) = panel_item.core_surface.upgrade() {
						let flex_vec = flexbuffers::Reader::get_root(data)?.get_vector()?;
						let button = flex_vec.index(0)?.get_u64()? as u32;
						let state = flex_vec.index(1)?.get_u64()? as u32;
						pointer.button(
							0,
							0,
							button,
							match state {
								0 => ButtonState::Released,
								1 => ButtonState::Pressed,
								_ => {
									bail!("Button state is out of bounds")
								}
							},
						);
						pointer.frame();
						core_surface.flush_clients();
					}
				}
			}
		}

		Ok(())
	}

	fn pointer_scroll_flex(node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		if let ItemType::Panel(panel_item) = &node.item.get().unwrap().specialization {
			if let Some(pointer) = panel_item.seat_data.pointer() {
				if *panel_item.seat_data.pointer_active.lock() {
					if let Some(core_surface) = panel_item.core_surface.upgrade() {
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
								pointer.axis_discrete(
									Axis::HorizontalScroll,
									axis_discrete_vec.x as i32,
								);
								pointer.axis_discrete(
									Axis::VerticalScroll,
									axis_discrete_vec.y as i32,
								);
							}
						}
						pointer.frame();
						core_surface.flush_clients();
					}
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
				if let Some(core_surface) = panel_item.core_surface.upgrade() {
					let mut keyboard_active = panel_item.seat_data.keyboard_active.lock();
					let active = flexbuffers::Reader::get_root(data)?.get_bool()?;
					if *keyboard_active != active {
						if active {
							keyboard.enter(0, &core_surface.wl_surface(), vec![]);
						} else {
							keyboard.leave(0, &core_surface.wl_surface());
						}
						*keyboard_active = active;
					}
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
		// Panel size
		{
			let mut size_vec = vec.start_vector();
			self.core_surface.upgrade().unwrap().with_data(|data| {
				size_vec.push(data.size.x);
				size_vec.push(data.size.y);
			});
		}

		// Cursor size and hotspot
		if let Some(cursor) = &*self.seat_data.cursor.lock() {
			let cursor = cursor.lock();
			if let Some(cursor_core_surface) = cursor.core_surface.upgrade() {
				if let Some(mapped_data) = &*cursor_core_surface.mapped_data.lock() {
					let mut cursor_vec = vec.start_vector();
					{
						let mut cursor_size_vec = cursor_vec.start_vector();
						cursor_size_vec.push(mapped_data.size.x);
						cursor_size_vec.push(mapped_data.size.y);
					}
					{
						let mut cursor_hotspot_vec = cursor_vec.start_vector();
						cursor_hotspot_vec.push(cursor.hotspot.x);
						cursor_hotspot_vec.push(cursor.hotspot.y);
					}
				} else {
					vec.push(());
				}
			} else {
				vec.push(());
			}
		} else {
			vec.push(());
		}
	}
}

pub fn register_panel_item_ui_flex(
	_node: &Node,
	calling_client: Arc<Client>,
	_data: &[u8],
) -> Result<()> {
	register_item_ui_flex(calling_client, &ITEM_TYPE_INFO_PANEL)
}

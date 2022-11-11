use super::{
	seat::{KeyboardInfo, SeatData},
	surface::CoreSurface,
};
use crate::{
	core::{
		client::{Client, INTERNAL_CLIENT},
		registry::Registry,
	},
	nodes::{
		items::{Item, ItemSpecialization, ItemType, TypeInfo},
		spatial::Spatial,
		Node,
	},
};
use anyhow::{anyhow, bail, Result};
use glam::Mat4;
use lazy_static::lazy_static;
use mint::Vector2;
use nanoid::nanoid;
use serde::Deserialize;
use smithay::{
	reexports::wayland_server::protocol::wl_pointer::{Axis, ButtonState},
	utils::Size,
	wayland::{
		compositor::SurfaceData,
		shell::xdg::{Configure, XdgToplevelSurfaceData},
	},
};
use stardust_xr::schemas::flex::{deserialize, serialize};
use std::sync::{Arc, Weak};
use xkbcommon::xkb::{self, ffi::XKB_KEYMAP_FORMAT_TEXT_V1, Keymap};

lazy_static! {
	pub static ref ITEM_TYPE_INFO_PANEL: TypeInfo = TypeInfo {
		type_name: "panel",
		aliased_local_signals: vec![
			"apply_surface_material",
			"apply_cursor_material",
			"pointer_deactivate",
			"pointer_scroll",
			"pointer_button",
			"pointer_motion",
			"keyboard_set_active",
			"keyboard_set_keyState",
			"keyboard_set_modifiers",
			"resize",
			"close",
		],
		aliased_local_methods: vec![],
		aliased_remote_signals: vec!["resize", "set_cursor",],
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
	pub fn create(core_surface: &Arc<CoreSurface>, seat_data: SeatData) -> Arc<Node> {
		let node = Arc::new(Node::create(
			&INTERNAL_CLIENT,
			"/item/panel/item",
			&nanoid!(),
			true,
		));
		Spatial::add_to(&node, None, Mat4::IDENTITY, false).unwrap();

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
			"apply_surface_material",
			PanelItem::apply_surface_material_flex,
		);
		node.add_local_signal(
			"apply_cursor_material",
			PanelItem::apply_cursor_material_flex,
		);
		node.add_local_signal("pointer_deactivate", PanelItem::pointer_deactivate_flex);
		node.add_local_signal("pointer_scroll", PanelItem::pointer_scroll_flex);
		node.add_local_signal("pointer_button", PanelItem::pointer_button_flex);
		node.add_local_signal("pointer_motion", PanelItem::pointer_motion_flex);
		node.add_local_signal(
			"keyboard_activate_string",
			PanelItem::keyboard_activate_string_flex,
		);
		node.add_local_signal(
			"keyboard_activate_names",
			PanelItem::keyboard_activate_names_flex,
		);
		node.add_local_signal("keyboard_deactivate", PanelItem::keyboard_deactivate_flex);
		node.add_local_signal("keyboard_key_state", PanelItem::keyboard_key_state_flex);
		node.add_local_signal("resize", PanelItem::resize_flex);
		node
	}

	pub fn from_node(node: &Node) -> Option<&PanelItem> {
		node.item.get().and_then(|item| match &item.specialization {
			ItemType::Panel(panel_item) => Some(panel_item),
			_ => None,
		})
	}

	fn apply_surface_material_flex(
		node: &Node,
		calling_client: Arc<Client>,
		data: &[u8],
	) -> Result<()> {
		#[derive(Deserialize)]
		struct SurfaceMaterialInfo<'a> {
			model_path: &'a str,
			idx: u32,
		}
		let info: SurfaceMaterialInfo = deserialize(data)?;
		let model_node = calling_client
			.scenegraph
			.get_node(info.model_path)
			.ok_or_else(|| anyhow!("Model node not found"))?;
		let model = model_node
			.model
			.get()
			.ok_or_else(|| anyhow!("Node is not a model"))?;

		if let ItemType::Panel(panel_item) = &node.item.get().unwrap().specialization {
			if let Some(core_surface) = panel_item.core_surface.upgrade() {
				core_surface.apply_material(model.clone(), info.idx);
			}
		}

		Ok(())
	}

	fn apply_cursor_material_flex(
		node: &Node,
		calling_client: Arc<Client>,
		data: &[u8],
	) -> Result<()> {
		let Some(panel_item) = PanelItem::from_node(node) else { return Ok(()) };
		let cursor = panel_item.seat_data.cursor.lock();
		let Some(cursor) = &*cursor else { return Ok(())};
		let Some(core_surface) = cursor.lock().core_surface.upgrade() else { return Ok(()) };

		#[derive(Deserialize)]
		struct SurfaceMaterialInfo<'a> {
			model_path: &'a str,
			idx: u32,
		}
		let info: SurfaceMaterialInfo = deserialize(data)?;
		let model_node = calling_client
			.scenegraph
			.get_node(info.model_path)
			.ok_or_else(|| anyhow!("Model node not found"))?;
		let model = model_node
			.model
			.get()
			.ok_or_else(|| anyhow!("Node is not a model"))?;

		core_surface.apply_material(model.clone(), info.idx);

		Ok(())
	}

	pub fn on_mapped(
		core_surface: &Arc<CoreSurface>,
		surface_data: &SurfaceData,
		seat_data: SeatData,
	) {
		if surface_data
			.data_map
			.get::<XdgToplevelSurfaceData>()
			.is_some()
		{
			surface_data
				.data_map
				.insert_if_missing_threadsafe(|| PanelItem::create(core_surface, seat_data));
		}
	}

	pub fn if_mapped(_core_surface: &Arc<CoreSurface>, surface_data: &SurfaceData) {
		let Some(panel_node) = surface_data.data_map.get::<Arc<Node>>() else { return };
		let Some(panel_item) = PanelItem::from_node(panel_node) else { return };

		panel_item.set_cursor();
	}

	pub fn ack_resize(&self, xdg_config: Configure) {
		let Configure::Toplevel(config) = xdg_config else { return };
		let Some(size) = config.state.size else { return };
		let Some(core_surface) = self.core_surface.upgrade() else { return };
		core_surface.with_data(|data| data.size = Vector2::from([size.w as u32, size.h as u32]));
		let _ = self
			.node
			.upgrade()
			.unwrap()
			.send_remote_signal("resize", &serialize((size.w, size.h)).unwrap());
	}

	pub fn set_cursor(&self) {
		let mut cursor_changed = self.seat_data.cursor_changed.lock();
		if !*cursor_changed {
			return;
		}
		let mut data = serialize(()).unwrap();

		let cursor = self.seat_data.cursor.lock();
		if let Some(cursor) = cursor.as_ref().map(|cursor| cursor.lock()) {
			if let Some(core_surface) = cursor.core_surface.upgrade() {
				if let Some(mapped_data) = &*core_surface.mapped_data.lock() {
					data = serialize((mapped_data.size, cursor.hotspot)).unwrap();
				}
			}
		}

		let _ = self
			.node
			.upgrade()
			.unwrap()
			.send_remote_signal("set_cursor", &data);
		*cursor_changed = false;
	}

	fn pointer_deactivate_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		_data: &[u8],
	) -> Result<()> {
		let Some(panel_item) = PanelItem::from_node(node) else { return Ok(()) };
		if !panel_item.seat_data.pointer_active() {
			return Ok(());
		}
		let Some(core_surface) = panel_item.core_surface.upgrade() else { return Ok(()) };
		let Some(wl_surface) = core_surface.wl_surface() else { return Ok(()) };
		let Some(pointer) = panel_item.seat_data.pointer() else { return Ok(()) };

		pointer.leave(0, &wl_surface);
		*panel_item.seat_data.pointer_active.lock() = false;
		pointer.frame();
		core_surface.flush_clients();

		Ok(())
	}

	fn pointer_motion_flex(node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		let Some(panel_item) = PanelItem::from_node(node) else { return Ok(()) };
		let Some(core_surface) = panel_item.core_surface.upgrade() else { return Ok(()) };
		let Some(wl_surface) = core_surface.wl_surface() else { return Ok(()) };
		let Some(pointer) = panel_item.seat_data.pointer() else { return Ok(()) };

		let Some(pointer_surface_size) =
			core_surface.with_data(|data| data.size) else { return Ok(()) };

		let mut position: Vector2<f64> = deserialize(data)?;
		position.x = position.x.clamp(0.0, pointer_surface_size.x as f64);
		position.y = position.y.clamp(0.0, pointer_surface_size.y as f64);
		if panel_item.seat_data.pointer_active() {
			pointer.motion(0, position.x, position.y);
		} else {
			pointer.enter(0, &wl_surface, position.x, position.y);
			*panel_item.seat_data.pointer_active.lock() = true;
		}
		pointer.frame();
		core_surface.flush_clients();

		Ok(())
	}

	fn pointer_button_flex(node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		let Some(panel_item) = PanelItem::from_node(node) else { return Ok(()) };
		if !panel_item.seat_data.pointer_active() {
			return Ok(());
		}
		let Some(core_surface) = panel_item.core_surface.upgrade() else { return Ok(()) };
		let Some(pointer) = panel_item.seat_data.pointer() else { return Ok(()) };

		let (button, state): (u32, u32) = deserialize(data)?;
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

		Ok(())
	}

	fn pointer_scroll_flex(node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		let Some(panel_item) = PanelItem::from_node(node) else { return Ok(()) };
		if !panel_item.seat_data.pointer_active() {
			return Ok(());
		}
		let Some(core_surface) = panel_item.core_surface.upgrade() else { return Ok(()) };
		let Some(pointer) = panel_item.seat_data.pointer() else { return Ok(()) };

		#[derive(Deserialize)]
		struct PointerScrollArgs {
			axis_continuous: Vector2<f32>,
			axis_discrete: Option<Vector2<f32>>,
		}
		let args: Option<PointerScrollArgs> = deserialize(data)?;

		match args {
			Some(args) => {
				pointer.axis(0, Axis::HorizontalScroll, args.axis_continuous.x as f64);
				pointer.axis(0, Axis::VerticalScroll, args.axis_continuous.y as f64);
				if let Some(axis_discrete_vec) = args.axis_discrete {
					pointer.axis_discrete(Axis::HorizontalScroll, axis_discrete_vec.x as i32);
					pointer.axis_discrete(Axis::VerticalScroll, axis_discrete_vec.y as i32);
				}
			}
			None => {
				pointer.axis_stop(0, Axis::HorizontalScroll);
				pointer.axis_stop(0, Axis::VerticalScroll);
			}
		};

		pointer.frame();
		core_surface.flush_clients();

		Ok(())
	}

	fn keyboard_activate_string_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		data: &[u8],
	) -> Result<()> {
		let context = xkb::Context::new(0);
		let keymap =
			Keymap::new_from_string(&context, deserialize(data)?, XKB_KEYMAP_FORMAT_TEXT_V1, 0)
				.ok_or_else(|| anyhow!("Keymap is not valid"))?;

		PanelItem::keyboard_activate_flex(node, &keymap)
	}

	fn keyboard_activate_names_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		data: &[u8],
	) -> Result<()> {
		#[derive(Deserialize)]
		struct Names<'a> {
			rules: &'a str,
			model: &'a str,
			layout: &'a str,
			variant: &'a str,
			options: Option<String>,
		}
		let names: Names = deserialize(data)?;
		let context = xkb::Context::new(0);
		let keymap = Keymap::new_from_names(
			&context,
			names.rules,
			names.model,
			names.layout,
			names.variant,
			names.options,
			XKB_KEYMAP_FORMAT_TEXT_V1,
		)
		.ok_or_else(|| anyhow!("Keymap is not valid"))?;

		PanelItem::keyboard_activate_flex(node, &keymap)
	}

	fn keyboard_activate_flex(node: &Node, keymap: &Keymap) -> Result<()> {
		let Some(panel_item) = PanelItem::from_node(node) else { return Ok(()) };
		let Some(core_surface) = panel_item.core_surface.upgrade() else { return Ok(()) };
		let Some(wl_surface) = core_surface.wl_surface() else { return Ok(()) };
		let Some(keyboard) = panel_item.seat_data.keyboard() else { return Ok(()) };

		let mut keyboard_info = panel_item.seat_data.keyboard_info.lock();
		if keyboard_info.is_none() {
			keyboard.enter(0, &wl_surface, vec![]);
			keyboard.repeat_info(0, 0);
		}
		keyboard_info.replace(KeyboardInfo::new(keymap));
		keyboard_info.as_ref().unwrap().keymap.send(keyboard)?;

		Ok(())
	}

	fn keyboard_deactivate_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		_data: &[u8],
	) -> Result<()> {
		let Some(panel_item) = PanelItem::from_node(node) else { return Ok(()) };
		let Some(core_surface) = panel_item.core_surface.upgrade() else { return Ok(()) };
		let Some(wl_surface) = core_surface.wl_surface() else { return Ok(()) };
		let Some(keyboard) = panel_item.seat_data.keyboard() else { return Ok(()) };

		let mut keyboard_info = panel_item.seat_data.keyboard_info.lock();
		if keyboard_info.is_some() {
			keyboard.leave(0, &wl_surface);
			*keyboard_info = None;
		}

		Ok(())
	}

	fn keyboard_key_state_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		data: &[u8],
	) -> Result<()> {
		let Some(panel_item) = PanelItem::from_node(node) else { return Ok(()) };
		let Some(keyboard) = panel_item.seat_data.keyboard() else { return Ok(()) };

		let mut keyboard_info = panel_item.seat_data.keyboard_info.lock();
		if let Some(keyboard_info) = &mut *keyboard_info {
			let (key, state): (u32, u32) = deserialize(data)?;
			keyboard_info.process(key, state, keyboard)?;
		}

		Ok(())
	}

	fn resize_flex(node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		let Some(panel_item) = PanelItem::from_node(node) else { return Ok(()) };
		let Some(core_surface) = panel_item.core_surface.upgrade() else { return Ok(()) };
		let Some(wl_surface) = core_surface.wl_surface() else { return Ok(()) };
		let size: Vector2<u32> = deserialize(data)?;

		let toplevel_surface = core_surface
			.wayland_state()
			.lock()
			.xdg_shell_state
			.toplevel_surfaces(|surfaces| {
				surfaces
					.iter()
					.find(|surf| surf.wl_surface().clone() == wl_surface)
					.cloned()
			});

		if let Some(toplevel_surface) = toplevel_surface {
			let mut size_set = false;
			toplevel_surface.with_pending_state(|state| {
				state.size = Some(Size::default());
				state.size.as_mut().unwrap().w = size.x as i32;
				state.size.as_mut().unwrap().h = size.y as i32;
				size_set = true;
			});
			if size_set {
				toplevel_surface.send_configure();
			}
		}

		Ok(())
	}
}
impl ItemSpecialization for PanelItem {
	fn serialize_start_data(&self, id: &str) -> Vec<u8> {
		// Panel size
		let panel_size = self
			.core_surface
			.upgrade()
			.unwrap()
			.with_data(|data| data.size);

		let cursor_lock = (*self.seat_data.cursor.lock()).clone();
		let cursor_size = cursor_lock
			.clone()
			.and_then(|cursor| cursor.lock().core_surface.upgrade())
			.and_then(|surf| surf.with_data(|data| data.size));
		let cursor_hotspot = cursor_lock.map(|cursor| cursor.lock().hotspot);

		serialize((id, (panel_size, cursor_size.zip(cursor_hotspot)))).unwrap()
	}
}

use super::{
	seat::{Cursor, KeyboardInfo, SeatData},
	surface::CoreSurface,
	xdg_shell::{XdgSurfaceData, XdgToplevelData},
};
use crate::{
	core::{
		client::{get_env, startup_settings, Client, INTERNAL_CLIENT},
		registry::Registry,
	},
	nodes::{
		items::{self, Item, ItemSpecialization, ItemType, TypeInfo},
		spatial::Spatial,
		Node,
	},
};
use color_eyre::eyre::{bail, eyre, Result};
use glam::Mat4;
use lazy_static::lazy_static;
use mint::Vector2;
use nanoid::nanoid;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use smithay::{
	reexports::{
		wayland_protocols::xdg::shell::server::xdg_toplevel::{
			XdgToplevel, EVT_CONFIGURE_BOUNDS_SINCE,
		},
		wayland_server::{
			backend::Credentials,
			protocol::{
				wl_pointer::{Axis, ButtonState},
				wl_surface::WlSurface,
			},
			Resource, Weak as WlWeak,
		},
	},
	wayland::compositor,
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
		aliased_remote_signals: vec!["commit_toplevel", "set_cursor",],
		ui: Default::default(),
		items: Registry::new(),
		acceptors: Registry::new(),
	};
}

#[derive(Debug, Clone, Serialize)]
pub struct ToplevelState {
	#[serde(skip_serializing)]
	pub mapped: bool,
	#[serde(skip_serializing)]
	pub parent: Option<WlWeak<XdgToplevel>>,
	pub title: String,
	pub app_id: String,
	pub size: Vector2<u32>,
	pub max_size: Vector2<u32>,
	pub min_size: Vector2<u32>,
	pub states: Vec<u8>,
	#[serde(skip_serializing)]
	pub queued_state: Option<Box<ToplevelState>>,
}
impl Default for ToplevelState {
	fn default() -> Self {
		Self {
			mapped: false,
			parent: None,
			title: String::default(),
			app_id: String::default(),
			size: Vector2::from([0; 2]),
			max_size: Vector2::from([0; 2]),
			min_size: Vector2::from([0; 2]),
			states: Vec::new(),
			queued_state: None,
		}
	}
}

pub struct PanelItem {
	node: Weak<Node>,
	client_credentials: Option<Credentials>,
	pub toplevel: WlWeak<XdgToplevel>,
	pub cursor: Mutex<Option<WlWeak<WlSurface>>>,
	seat_data: SeatData,
}
impl PanelItem {
	pub fn create(
		toplevel: XdgToplevel,
		client_credentials: Option<Credentials>,
		seat_data: SeatData,
	) -> (Arc<Node>, Arc<PanelItem>) {
		let node = Arc::new(Node::create(
			&INTERNAL_CLIENT,
			"/item/panel/item",
			&nanoid!(),
			true,
		));
		let spatial = Spatial::add_to(&node, None, Mat4::IDENTITY, false).unwrap();
		let panel_item = Arc::new(PanelItem {
			node: Arc::downgrade(&node),
			client_credentials,
			toplevel: toplevel.downgrade(),
			cursor: Mutex::new(None),
			seat_data,
		});
		let _ = panel_item
			.seat_data
			.panel_item
			.set(Arc::downgrade(&panel_item));

		let item = Item::add_to(
			&node,
			&ITEM_TYPE_INFO_PANEL,
			ItemType::Panel(panel_item.clone()),
		);
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
		node.add_local_signal("configure_toplevel", PanelItem::configure_toplevel_flex);

		if let Some(startup_settings) = panel_item
			.client_credentials
			.and_then(|cred| get_env(cred.pid).ok())
			.and_then(|env| startup_settings(&env))
		{
			spatial.set_local_transform(startup_settings.transform);
			if let Some(acceptor) = startup_settings
				.acceptors
				.get(&*ITEM_TYPE_INFO_PANEL)
				.and_then(|acc| acc.upgrade())
			{
				items::capture(&item, &acceptor);
			}
		}

		(node, panel_item)
	}

	pub fn from_node(node: &Node) -> Option<&PanelItem> {
		node.item.get().and_then(|item| match &item.specialization {
			ItemType::Panel(panel_item) => Some(&**panel_item),
			_ => None,
		})
	}

	fn toplevel_surface_data(&self) -> Option<XdgSurfaceData> {
		Some(
			self.toplevel
				.upgrade()
				.ok()?
				.data::<XdgToplevelData>()?
				.xdg_surface_data
				.clone(),
		)
	}
	pub fn toplevel_state(&self) -> Option<Arc<Mutex<ToplevelState>>> {
		Some(
			self.toplevel
				.upgrade()
				.ok()?
				.data::<XdgToplevelData>()?
				.state
				.clone(),
		)
	}
	fn toplevel_wl_surface(&self) -> Option<WlSurface> {
		self.toplevel_surface_data()?.wl_surface.upgrade().ok()
	}
	fn core_surface(&self) -> Option<Arc<CoreSurface>> {
		compositor::with_states(&self.toplevel_wl_surface()?, |data| {
			data.data_map.get::<Arc<CoreSurface>>().cloned()
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
			.ok_or_else(|| eyre!("Model node not found"))?;
		let model = model_node
			.model
			.get()
			.ok_or_else(|| eyre!("Node is not a model"))?;

		if let ItemType::Panel(panel_item) = &node.item.get().unwrap().specialization {
			if let Some(core_surface) = panel_item.core_surface() {
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
		let Some(cursor) = panel_item.seat_data.cursor() else { return Ok(())};
		let Some(core_surface) = CoreSurface::from_wl_surface(&cursor) else { return Ok(()) };

		#[derive(Deserialize)]
		struct SurfaceMaterialInfo<'a> {
			model_path: &'a str,
			idx: u32,
		}
		let info: SurfaceMaterialInfo = deserialize(data)?;
		let model_node = calling_client
			.scenegraph
			.get_node(info.model_path)
			.ok_or_else(|| eyre!("Model node not found"))?;
		let model = model_node
			.model
			.get()
			.ok_or_else(|| eyre!("Node is not a model"))?;

		core_surface.apply_material(model.clone(), info.idx);

		Ok(())
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
		let Some(core_surface) = panel_item.core_surface() else { return Ok(()) };
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
		let Some(core_surface) = panel_item.core_surface() else { return Ok(()) };
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
		let Some(core_surface) = panel_item.core_surface() else { return Ok(()) };
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
		let Some(core_surface) = panel_item.core_surface() else { return Ok(()) };
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
				.ok_or_else(|| eyre!("Keymap is not valid"))?;

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
		.ok_or_else(|| eyre!("Keymap is not valid"))?;

		PanelItem::keyboard_activate_flex(node, &keymap)
	}

	fn keyboard_activate_flex(node: &Node, keymap: &Keymap) -> Result<()> {
		let Some(panel_item) = PanelItem::from_node(node) else { return Ok(()) };
		let Some(core_surface) = panel_item.core_surface() else { return Ok(()) };
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
		let Some(core_surface) = panel_item.core_surface() else { return Ok(()) };
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

	fn configure_toplevel_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		data: &[u8],
	) -> Result<()> {
		let Some(panel_item) = PanelItem::from_node(node) else { return Ok(()) };
		let Ok(xdg_toplevel) = panel_item.toplevel.upgrade() else { return Ok(()) };
		let Some(xdg_surface) = panel_item.toplevel_surface_data().and_then(|d| d.xdg_surface.upgrade().ok()) else { return Ok(()) };

		#[derive(Deserialize)]
		struct ConfigureToplevelInfo {
			size: Option<Vector2<u32>>,
			states: Vec<u8>,
			bounds: Option<Vector2<u32>>,
		}

		let info: ConfigureToplevelInfo = deserialize(data)?;
		if let Some(xdg_state) = panel_item.toplevel_state() {
			xdg_state.lock().queued_state.as_mut().unwrap().states = info.states.clone();
		}
		if let Some(bounds) = info.bounds {
			if xdg_toplevel.version() > EVT_CONFIGURE_BOUNDS_SINCE {
				xdg_toplevel.configure_bounds(bounds.x as i32, bounds.y as i32);
			}
		}
		let size = info.size.unwrap_or(Vector2::from([0; 2]));
		xdg_toplevel.configure(size.x as i32, size.y as i32, info.states);
		xdg_surface.configure(0);

		Ok(())
	}

	pub fn commit_toplevel(&self) {
		let mapped = self.core_surface().map(|c| c.mapped()).unwrap_or(false);
		let Some(state) = self.toplevel_state() else { return };
		let Some(surface_data) = self.toplevel_surface_data() else { return };
		let mut state = state.lock();
		{
			let queued_state = state.queued_state.as_mut().unwrap();
			queued_state.mapped = mapped;
			queued_state.size = *surface_data.size.lock();
		}

		let Some(node) = self.node.upgrade() else { return };
		let queued_state = state.queued_state.take().unwrap();
		*state = (*queued_state).clone();
		state.queued_state = Some(queued_state);

		let _ = node.send_remote_signal("commit_toplevel", &serialize(&*state).unwrap());
	}

	pub fn set_cursor(&self, surface: Option<&WlSurface>, hotspot_x: i32, hotspot_y: i32) {
		let Some(node) = self.node.upgrade() else { return };
		let mut data = serialize(()).unwrap();

		let cursor_size = surface
			.and_then(|c| CoreSurface::from_wl_surface(c))
			.and_then(|c| c.with_data(|data| data.size));

		if let Some(size) = cursor_size {
			data = serialize((size, (hotspot_x, hotspot_y))).unwrap();
		}

		let _ = node.send_remote_signal("set_cursor", &data);
	}
}
impl ItemSpecialization for PanelItem {
	fn serialize_start_data(&self, id: &str) -> Vec<u8> {
		let cursor = self.cursor.lock().as_ref().and_then(|c| c.upgrade().ok());
		let cursor_size = cursor
			.as_ref()
			.and_then(|c| CoreSurface::from_wl_surface(&c))
			.and_then(|c| c.with_data(|data| data.size));
		let cursor_hotspot = cursor
			.and_then(|c| {
				compositor::with_states(&c, |data| data.data_map.get::<Arc<Cursor>>().cloned())
			})
			.map(|cursor| cursor.hotspot);

		let toplevel_state = self.toplevel_state();
		let toplevel_state = toplevel_state.as_ref().map(|state| state.lock());
		serialize((
			id,
			(
				toplevel_state.and_then(|state| state.mapped.then_some(state.clone())),
				cursor_size.zip(cursor_hotspot),
			),
		))
		.unwrap()
	}
}

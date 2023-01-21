use super::{
	seat::{Cursor, SeatData},
	surface::CoreSurface,
	xdg_shell::{XdgSurfaceData, XdgToplevelData},
	SERIAL_COUNTER,
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
	wayland::seat::{KeyboardEvent, PointerEvent},
};
use color_eyre::eyre::{eyre, Result};
use glam::Mat4;
use lazy_static::lazy_static;
use mint::Vector2;
use nanoid::nanoid;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use smithay::{
	reexports::{
		wayland_protocols::xdg::shell::server::xdg_toplevel::{
			XdgToplevel, EVT_CONFIGURE_BOUNDS_SINCE, EVT_WM_CAPABILITIES_SINCE,
		},
		wayland_server::{
			backend::Credentials, protocol::wl_surface::WlSurface, Resource, Weak as WlWeak,
		},
	},
	wayland::compositor,
};
use stardust_xr::schemas::flex::{deserialize, serialize};
use std::sync::{Arc, Weak};
use tracing::debug;
use xkbcommon::xkb::{self, ffi::XKB_KEYMAP_FORMAT_TEXT_V1, Keymap};

lazy_static! {
	pub static ref ITEM_TYPE_INFO_PANEL: TypeInfo = TypeInfo {
		type_name: "panel",
		aliased_local_signals: vec![
			"apply_cursor_material",
			"apply_toplevel_material",
			"configure_toplevel",
			"set_toplevel_capabilities",
			"pointer_set_active",
			"pointer_scroll",
			"pointer_button",
			"pointer_motion",
			"keyboard_set_active",
			"keyboard_key",
			"keyboard_set_keymap_names",
			"keyboard_set_keymap_string",
			"close",
		],
		aliased_local_methods: vec![],
		aliased_remote_signals: vec!["commit_toplevel", "recommend_toplevel_state", "set_cursor"],
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
	pub title: Option<String>,
	pub app_id: Option<String>,
	pub size: Vector2<u32>,
	pub max_size: Option<Vector2<u32>>,
	pub min_size: Option<Vector2<u32>>,
	pub states: Vec<u32>,
	#[serde(skip_serializing)]
	pub queued_state: Option<Box<ToplevelState>>,
}
impl Default for ToplevelState {
	fn default() -> Self {
		Self {
			mapped: false,
			parent: None,
			title: None,
			app_id: None,
			size: Vector2::from([0; 2]),
			max_size: None,
			min_size: None,
			states: Vec::new(),
			queued_state: None,
		}
	}
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(tag = "type", content = "content")]
pub enum RecommendedState {
	Maximize(bool),
	Fullscreen(bool),
	Minimize,
	Move,
	Resize(u32),
}

pub struct PanelItem {
	node: Weak<Node>,
	client_credentials: Option<Credentials>,
	toplevel: WlWeak<XdgToplevel>,
	pub cursor: Mutex<Option<WlWeak<WlSurface>>>,
	seat_data: SeatData,
}
impl PanelItem {
	pub fn create(
		toplevel: XdgToplevel,
		wl_surface: WlSurface,
		client_credentials: Option<Credentials>,
		seat_data: SeatData,
	) -> (Arc<Node>, Arc<PanelItem>) {
		debug!(?toplevel, ?client_credentials, "Create panel item");
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

		panel_item
			.seat_data
			.new_panel_item(&panel_item, &toplevel, &wl_surface);

		let item = Item::add_to(
			&node,
			&ITEM_TYPE_INFO_PANEL,
			ItemType::Panel(panel_item.clone()),
		);
		node.add_local_signal(
			"apply_toplevel_material",
			PanelItem::apply_toplevel_material_flex,
		);
		node.add_local_signal("configure_toplevel", PanelItem::configure_toplevel_flex);
		if toplevel.version() >= EVT_WM_CAPABILITIES_SINCE {
			node.add_local_signal(
				"set_toplevel_capabilities",
				PanelItem::set_toplevel_capabilities_flex,
			);
		}
		node.add_local_signal(
			"apply_cursor_material",
			PanelItem::apply_cursor_material_flex,
		);
		node.add_local_signal("pointer_set_active", PanelItem::pointer_set_active_flex);
		node.add_local_signal("pointer_scroll", PanelItem::pointer_scroll_flex);
		node.add_local_signal("pointer_button", PanelItem::pointer_button_flex);
		node.add_local_signal("pointer_motion", PanelItem::pointer_motion_flex);

		node.add_local_signal("keyboard_set_active", PanelItem::keyboard_set_active_flex);
		node.add_local_signal(
			"keyboard_set_keymap_string",
			PanelItem::keyboard_set_keymap_string_flex,
		);
		node.add_local_signal(
			"keyboard_set_keymap_names",
			PanelItem::keyboard_set_keymap_names_flex,
		);
		node.add_local_signal("keyboard_key", PanelItem::keyboard_key_flex);

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
	fn toplevel_state(&self) -> Option<Arc<Mutex<ToplevelState>>> {
		Some(
			self.toplevel
				.upgrade()
				.ok()?
				.data::<XdgToplevelData>()?
				.state
				.clone(),
		)
	}
	pub fn toplevel_wl_surface(&self) -> Option<WlSurface> {
		self.toplevel_surface_data()?.wl_surface.upgrade().ok()
	}
	fn core_surface(&self) -> Option<Arc<CoreSurface>> {
		compositor::with_states(&self.toplevel_wl_surface()?, |data| {
			data.data_map.get::<Arc<CoreSurface>>().cloned()
		})
	}
	fn flush_clients(&self) {
		if let Some(core_surface) = self.core_surface() {
			core_surface.flush_clients();
		}
	}

	fn apply_toplevel_material_flex(
		node: &Node,
		calling_client: Arc<Client>,
		data: &[u8],
	) -> Result<()> {
		#[derive(Debug, Deserialize)]
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
		debug!(?info, "Apply toplevel material");

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
		let Some(cursor) = panel_item.cursor.lock().as_ref().and_then(|c| c.upgrade().ok()) else { return Ok(())};
		let Some(core_surface) = CoreSurface::from_wl_surface(&cursor) else { return Ok(()) };

		#[derive(Debug, Deserialize)]
		struct SurfaceMaterialInfo<'a> {
			model_path: &'a str,
			idx: u32,
		}
		let info: SurfaceMaterialInfo = deserialize(data)?;
		debug!(?cursor, ?info, "Apply cursor material");
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

	fn pointer_motion_flex(node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		let Some(panel_item) = PanelItem::from_node(node) else { return Ok(()) };
		let Ok(toplevel) = panel_item.toplevel.upgrade() else { return Ok(()) };
		debug!(?toplevel, "Pointer deactivate");

		panel_item
			.seat_data
			.pointer_event(&toplevel, PointerEvent::Motion(deserialize(data)?));
		panel_item.flush_clients();

		Ok(())
	}
	fn pointer_button_flex(node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		let Some(panel_item) = PanelItem::from_node(node) else { return Ok(()) };
		let Ok(toplevel) = panel_item.toplevel.upgrade() else { return Ok(()) };

		let (button, state): (u32, u32) = deserialize(data)?;
		debug!(button, state, "Pointer button");

		panel_item
			.seat_data
			.pointer_event(&toplevel, PointerEvent::Button { button, state });
		panel_item.flush_clients();

		Ok(())
	}
	fn pointer_scroll_flex(node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		let Some(panel_item) = PanelItem::from_node(node) else { return Ok(()) };
		let Ok(toplevel) = panel_item.toplevel.upgrade() else { return Ok(()) };

		#[derive(Debug, Deserialize)]
		struct PointerScrollArgs {
			axis_continuous: Vector2<f32>,
			axis_discrete: Option<Vector2<f32>>,
		}
		let args: Option<PointerScrollArgs> = deserialize(data)?;

		debug!(?args, "Pointer scroll");

		panel_item.seat_data.pointer_event(
			&toplevel,
			PointerEvent::Scroll {
				axis_continuous: args.as_ref().map(|a| a.axis_continuous),
				axis_discrete: args.and_then(|a| a.axis_discrete),
			},
		);
		panel_item.flush_clients();

		Ok(())
	}
	fn pointer_set_active_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		data: &[u8],
	) -> Result<()> {
		let Some(panel_item) = PanelItem::from_node(node) else { return Ok(()) };
		let Ok(toplevel) = panel_item.toplevel.upgrade() else { return Ok(()) };
		let active: bool = deserialize(data)?;
		debug!(?toplevel, active, "Pointer set active");

		panel_item.seat_data.set_pointer_active(&toplevel, active);
		panel_item.flush_clients();

		Ok(())
	}

	fn keyboard_set_keymap_string_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		data: &[u8],
	) -> Result<()> {
		let context = xkb::Context::new(0);
		let keymap =
			Keymap::new_from_string(&context, deserialize(data)?, XKB_KEYMAP_FORMAT_TEXT_V1, 0)
				.ok_or_else(|| eyre!("Keymap is not valid"))?;

		PanelItem::keyboard_set_keymap_flex(node, &keymap)
	}
	fn keyboard_set_keymap_names_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		data: &[u8],
	) -> Result<()> {
		#[derive(Debug, Deserialize)]
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

		PanelItem::keyboard_set_keymap_flex(node, &keymap)
	}
	fn keyboard_set_keymap_flex(node: &Node, keymap: &Keymap) -> Result<()> {
		let Some(panel_item) = PanelItem::from_node(node) else { return Ok(()) };
		let Ok(toplevel) = panel_item.toplevel.upgrade() else { return Ok(()) };
		debug!(?toplevel, "Keyboard set keymap");

		panel_item.seat_data.set_keymap(&toplevel, keymap);

		Ok(())
	}

	fn keyboard_set_active_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		data: &[u8],
	) -> Result<()> {
		let Some(panel_item) = PanelItem::from_node(node) else { return Ok(()) };
		let Ok(toplevel) = panel_item.toplevel.upgrade() else { return Ok(()) };
		let active: bool = deserialize(data)?;
		debug!(?toplevel, active, "Keyboard set active");

		panel_item.seat_data.set_keyboard_active(&toplevel, active);

		Ok(())
	}

	fn keyboard_key_flex(node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		let Some(panel_item) = PanelItem::from_node(node) else { return Ok(()) };
		let Ok(toplevel) = panel_item.toplevel.upgrade() else { return Ok(()) };
		let (key, state): (u32, u32) = deserialize(data)?;
		debug!(key, state, "Set keyboard key state");

		panel_item
			.seat_data
			.keyboard_event(&toplevel, KeyboardEvent::Key { key, state });

		Ok(())
	}

	fn configure_toplevel_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		data: &[u8],
	) -> Result<()> {
		let Some(panel_item) = PanelItem::from_node(node) else { return Ok(()) };
		let Some(core_surface) = panel_item.core_surface() else { return Ok(()) };
		let Ok(xdg_toplevel) = panel_item.toplevel.upgrade() else { return Ok(()) };
		let Some(xdg_surface) = panel_item.toplevel_surface_data().and_then(|d| d.xdg_surface.upgrade().ok()) else { return Ok(()) };

		#[derive(Debug, Deserialize)]
		struct ConfigureToplevelInfo {
			size: Option<Vector2<u32>>,
			states: Vec<u32>,
			bounds: Option<Vector2<u32>>,
		}

		let info: ConfigureToplevelInfo = deserialize(data)?;
		debug!(info = ?&info, "Configure toplevel info");
		if let Some(xdg_state) = panel_item.toplevel_state() {
			xdg_state.lock().queued_state.as_mut().unwrap().states = info.states.clone();
		}
		if let Some(bounds) = info.bounds {
			if xdg_toplevel.version() > EVT_CONFIGURE_BOUNDS_SINCE {
				xdg_toplevel.configure_bounds(bounds.x as i32, bounds.y as i32);
			}
		}
		let size = info.size.unwrap_or(Vector2::from([0; 2]));
		xdg_toplevel.configure(
			size.x as i32,
			size.y as i32,
			info.states
				.into_iter()
				.flat_map(|state| state.to_ne_bytes())
				.collect::<Vec<_>>(),
		);
		xdg_surface.configure(SERIAL_COUNTER.inc());
		core_surface.flush_clients();

		Ok(())
	}

	fn set_toplevel_capabilities_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		data: &[u8],
	) -> Result<()> {
		let Some(panel_item) = PanelItem::from_node(node) else { return Ok(()) };
		let Some(core_surface) = panel_item.core_surface() else { return Ok(()) };
		let Ok(xdg_toplevel) = panel_item.toplevel.upgrade() else { return Ok(()) };
		let Some(xdg_surface) = panel_item.toplevel_surface_data().and_then(|d| d.xdg_surface.upgrade().ok()) else { return Ok(()) };

		let capabilities: Vec<u8> = deserialize(data)?;
		debug!("Set toplevel capabilities");
		xdg_toplevel.wm_capabilities(capabilities);
		xdg_surface.configure(SERIAL_COUNTER.inc());
		core_surface.flush_clients();

		Ok(())
	}

	pub fn commit_toplevel(&self) {
		let mapped_size = self.core_surface().and_then(|c| c.size());
		let Some(state) = self.toplevel_state() else { return };
		let mut state = state.lock();
		let mut queued_state = state.queued_state.take().unwrap();
		queued_state.mapped =
			mapped_size.is_some() && mapped_size.unwrap().x > 0 && mapped_size.unwrap().y > 0;
		if let Some(size) = mapped_size {
			queued_state.size = size;
		}
		*state = (*queued_state).clone();
		state.queued_state = Some(queued_state);

		debug!(state = ?&state.mapped.then_some(&*state), "Commit toplevel");
		let Some(node) = self.node.upgrade() else { return };
		let _ = node.send_remote_signal(
			"commit_toplevel",
			&serialize(&state.mapped.then_some(&*state)).unwrap(),
		);
	}

	pub fn recommend_toplevel_state(&self, state: RecommendedState) {
		let Some(node) = self.node.upgrade() else { return };
		let data = serialize(state).unwrap();
		debug!(?state, "Recommend toplevel state");

		let _ = node.send_remote_signal("recommend_toplevel_state", &data);
	}

	pub fn set_cursor(&self, surface: Option<&WlSurface>, hotspot_x: i32, hotspot_y: i32) {
		let Some(node) = self.node.upgrade() else { return };
		debug!(?surface, hotspot_x, hotspot_y, "Set cursor size");
		let mut data = serialize(()).unwrap();

		let cursor_size = surface
			.and_then(|c| CoreSurface::from_wl_surface(c))
			.and_then(|c| c.size());

		if let Some(size) = cursor_size {
			data = serialize((size, (hotspot_x, hotspot_y))).unwrap();
		}

		let _ = node.send_remote_signal("set_cursor", &data);
		*self.cursor.lock() = surface.map(|surf| surf.downgrade());
	}

	pub fn on_drop(&self) {
		let Ok(toplevel) = self.toplevel.upgrade() else { return; };
		self.seat_data.drop_panel_item(&toplevel);

		debug!("Drop panel item");
	}
}
impl ItemSpecialization for PanelItem {
	fn serialize_start_data(&self, id: &str) -> Vec<u8> {
		let cursor = self.cursor.lock().as_ref().and_then(|c| c.upgrade().ok());
		let cursor_size = cursor
			.as_ref()
			.and_then(|c| CoreSurface::from_wl_surface(&c))
			.and_then(|c| c.size());
		let cursor_hotspot = cursor
			.and_then(|c| {
				compositor::with_states(&c, |data| data.data_map.get::<Arc<Cursor>>().cloned())
			})
			.map(|cursor| cursor.hotspot);

		let toplevel_state = self.toplevel_state();
		let toplevel_state = toplevel_state.as_ref().map(|state| state.lock());
		let toplevel_state = toplevel_state.and_then(|state| {
			(state.mapped && state.size.x > 0 && state.size.y > 0).then_some(state.clone())
		});
		serialize((id, (toplevel_state, cursor_size.zip(cursor_hotspot)))).unwrap()
	}
}

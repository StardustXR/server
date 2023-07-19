use super::{
	seat::{Cursor, SeatData},
	surface::CoreSurface,
	xdg_shell::{PopupData, ToplevelData, XdgSurfaceData},
	SERIAL_COUNTER,
};
use crate::{
	core::{
		client::{get_env, startup_settings, Client, INTERNAL_CLIENT},
		registry::Registry,
	},
	nodes::{
		drawable::Drawable,
		items::{self, Item, ItemSpecialization, ItemType, TypeInfo},
		spatial::Spatial,
		Node,
	},
	wayland::seat::{KeyboardEvent, PointerEvent},
};
use color_eyre::eyre::{bail, eyre, Result};
use glam::Mat4;
use lazy_static::lazy_static;
use mint::Vector2;
use nanoid::nanoid;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use serde::{
	de::{Deserializer, Error, SeqAccess, Visitor},
	ser::Serializer,
	Deserialize, Serialize,
};
use smithay::{
	reexports::{
		wayland_protocols::xdg::shell::server::{
			xdg_popup::XdgPopup,
			xdg_surface::XdgSurface,
			xdg_toplevel::{XdgToplevel, EVT_CONFIGURE_BOUNDS_SINCE, EVT_WM_CAPABILITIES_SINCE},
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
			"apply_surface_material",
			"configure_toplevel",
			"set_toplevel_capabilities",
			"pointer_scroll",
			"pointer_button",
			"pointer_motion",
			"keyboard_key",
			"keyboard_set_keymap_names",
			"keyboard_set_keymap_string",
			"close",
		],
		aliased_local_methods: vec![],
		aliased_remote_signals: vec![
			"commit_toplevel",
			"recommend_toplevel_state",
			"set_cursor",
			"new_popup",
			"reposition_popup",
			"drop_popup",
		],
		ui: Default::default(),
		items: Registry::new(),
		acceptors: Registry::new(),
	};
}

/// An ID for a surface inside this panel item
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum SurfaceID {
	Cursor,
	Toplevel,
	Popup(String),
}
impl Default for SurfaceID {
	fn default() -> Self {
		Self::Toplevel
	}
}

impl<'de> serde::Deserialize<'de> for SurfaceID {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		deserializer.deserialize_seq(SurfaceIDVisitor)
	}
}

struct SurfaceIDVisitor;

impl<'de> Visitor<'de> for SurfaceIDVisitor {
	type Value = SurfaceID;

	fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		f.write_str("idk")
	}

	fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
		let Some(discrim) = seq.next_element()? else {
            return Err(A::Error::missing_field("discrim"));
        };

		// idk if you wanna check for extraneous elements
		// I didn't bother

		match discrim {
			"Cursor" => Ok(SurfaceID::Cursor),
			"Toplevel" => Ok(SurfaceID::Toplevel),
			"Popup" => {
				let Some(text) = seq.next_element()? else {
                    return Err(A::Error::missing_field("popup_text"));
                };
				Ok(SurfaceID::Popup(text))
			}
			_ => Err(A::Error::unknown_variant(
				discrim,
				&["Cursor", "Toplevel", "Popup"],
			)),
		}
	}
}

impl serde::Serialize for SurfaceID {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		match self {
			Self::Cursor => ["Cursor"].serialize(serializer),
			Self::Toplevel => ["Toplevel"].serialize(serializer),
			Self::Popup(text) => ["Popup", text].serialize(serializer),
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
	pub uid: String,
	node: Weak<Node>,
	cursor: Mutex<Option<WlWeak<WlSurface>>>,
	pub seat_data: Arc<SeatData>,
	toplevel: WlWeak<XdgToplevel>,
	popups: Mutex<FxHashMap<String, WlWeak<XdgPopup>>>,
	pointer_grab: Mutex<Option<SurfaceID>>,
	keyboard_grab: Mutex<Option<SurfaceID>>,
}
impl PanelItem {
	pub fn create(
		toplevel: XdgToplevel,
		wl_surface: WlSurface,
		client_credentials: Option<Credentials>,
		seat_data: Arc<SeatData>,
	) -> (Arc<Node>, Arc<PanelItem>) {
		debug!(?toplevel, ?client_credentials, "Create panel item");

		let startup_settings = client_credentials
			.and_then(|cred| get_env(cred.pid).ok())
			.and_then(|env| startup_settings(&env));

		let uid = nanoid!();
		let node = Arc::new(Node::create(
			&INTERNAL_CLIENT,
			"/item/panel/item",
			&uid,
			true,
		));
		let spatial = Spatial::add_to(&node, None, Mat4::IDENTITY, false).unwrap();
		let panel_item = Arc::new(PanelItem {
			uid: uid.clone(),
			node: Arc::downgrade(&node),
			cursor: Mutex::new(None),
			seat_data,
			toplevel: toplevel.downgrade(),
			popups: Mutex::new(FxHashMap::default()),
			pointer_grab: Mutex::new(None),
			keyboard_grab: Mutex::new(None),
		});

		if let Some(startup_settings) = &startup_settings {
			spatial.set_local_transform(
				spatial.global_transform().inverse() * startup_settings.transform,
			);
		}

		panel_item
			.seat_data
			.new_surface(&wl_surface, Arc::downgrade(&panel_item));

		let item = Item::add_to(
			&node,
			uid,
			&ITEM_TYPE_INFO_PANEL,
			ItemType::Panel(panel_item.clone()),
		);

		if let Some(startup_settings) = &startup_settings {
			if let Some(acceptor) = startup_settings
				.acceptors
				.get(&*ITEM_TYPE_INFO_PANEL)
				.and_then(|acc| acc.upgrade())
			{
				items::capture(&item, &acceptor);
			}
		}
		node.add_local_signal(
			"apply_surface_material",
			PanelItem::apply_surface_material_flex,
		);
		node.add_local_signal("configure_toplevel", PanelItem::configure_toplevel_flex);
		node.add_local_signal(
			"set_toplevel_capabilities",
			PanelItem::set_toplevel_capabilities_flex,
		);
		node.add_local_signal("pointer_scroll", PanelItem::pointer_scroll_flex);
		node.add_local_signal("pointer_button", PanelItem::pointer_button_flex);
		node.add_local_signal("pointer_motion", PanelItem::pointer_motion_flex);

		node.add_local_signal(
			"keyboard_set_keymap_string",
			PanelItem::keyboard_set_keymap_string_flex,
		);
		node.add_local_signal(
			"keyboard_set_keymap_names",
			PanelItem::keyboard_set_keymap_names_flex,
		);
		node.add_local_signal("keyboard_key", PanelItem::keyboard_key_flex);

		(node, panel_item)
	}

	pub fn from_node(node: &Node) -> Option<Arc<PanelItem>> {
		let ItemType::Panel(panel_item) = &node.item.get()?.specialization else {return None};
		Some(panel_item.clone())
	}

	fn toplevel(&self) -> XdgToplevel {
		self.toplevel.upgrade().unwrap()
	}
	fn toplevel_xdg_surface(&self) -> XdgSurface {
		let toplevel = self.toplevel();
		let data = ToplevelData::get(&toplevel).lock();
		data.xdg_surface()
	}
	fn toplevel_wl_surface(&self) -> WlSurface {
		XdgSurfaceData::get(&self.toplevel_xdg_surface())
			.lock()
			.wl_surface()
	}
	fn core_surface(&self) -> Option<Arc<CoreSurface>> {
		compositor::with_states(&self.toplevel_wl_surface(), |data| {
			data.data_map.get::<Arc<CoreSurface>>().cloned()
		})
	}
	fn flush_clients(&self) {
		if let Some(core_surface) = self.core_surface() {
			core_surface.flush_clients();
		}
	}
	fn wl_surface_from_id(&self, id: &SurfaceID) -> Option<WlSurface> {
		match id {
			SurfaceID::Cursor => self.cursor.lock().clone()?.upgrade().ok(),
			SurfaceID::Toplevel => Some(self.toplevel_wl_surface()),
			SurfaceID::Popup(popup) => {
				let popups = self.popups.lock();
				let popup = popups.get(popup)?.upgrade().ok()?;
				let surf = PopupData::get(&popup).lock().wl_surface();
				Some(surf)
			}
		}
	}
	fn wl_surface_from_id_result(&self, id: &SurfaceID) -> Result<WlSurface> {
		self.wl_surface_from_id(id)
			.ok_or(eyre!("Surface with ID not found"))
	}

	fn apply_surface_material_flex(
		node: &Node,
		calling_client: Arc<Client>,
		data: &[u8],
	) -> Result<()> {
		let Some(panel_item) = PanelItem::from_node(node) else { return Ok(()) };

		#[derive(Debug, Deserialize)]
		struct SurfaceMaterialInfo<'a> {
			surface: SurfaceID,
			model_node_path: &'a str,
		}

		let info: SurfaceMaterialInfo = deserialize(data)?;

		let Some(wl_surface) = panel_item.wl_surface_from_id(&info.surface) else { return Ok(()) };
		let Some(core_surface) = CoreSurface::from_wl_surface(&wl_surface) else { return Ok(()) };

		let model_node = calling_client
			.scenegraph
			.get_node(info.model_node_path)
			.ok_or_else(|| eyre!("Model node not found"))?;
		let Some(Drawable::ModelPart(model_node)) = model_node.drawable.get() else {bail!("Node is not a model")};
		debug!(?info, "Apply surface material");

		core_surface.apply_material(model_node);

		Ok(())
	}

	fn pointer_motion_flex(node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		let Some(panel_item) = PanelItem::from_node(node) else { return Ok(()) };

		let (surface_id, position): (SurfaceID, Vector2<f64>) = deserialize(data)?;
		let wl_surface = panel_item.wl_surface_from_id_result(&surface_id)?;
		debug!(?surface_id, ?position, "Pointer deactivate");

		panel_item
			.seat_data
			.pointer_event(&wl_surface, PointerEvent::Motion(position));
		panel_item.flush_clients();

		Ok(())
	}
	fn pointer_button_flex(node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		let Some(panel_item) = PanelItem::from_node(node) else { return Ok(()) };

		let (surface_id, button, state): (SurfaceID, u32, u32) = deserialize(data)?;
		let wl_surface = panel_item.wl_surface_from_id_result(&surface_id)?;
		debug!(?surface_id, button, state, "Pointer button");

		panel_item
			.seat_data
			.pointer_event(&wl_surface, PointerEvent::Button { button, state });
		panel_item.flush_clients();

		Ok(())
	}
	fn pointer_scroll_flex(node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		let Some(panel_item) = PanelItem::from_node(node) else { return Ok(()) };

		#[derive(Debug, Deserialize)]
		struct PointerScrollInfo {
			surface_id: SurfaceID,
			axis_continuous: Option<Vector2<f32>>,
			axis_discrete: Option<Vector2<f32>>,
		}
		let info: PointerScrollInfo = deserialize(data)?;
		let wl_surface = panel_item.wl_surface_from_id_result(&info.surface_id)?;

		debug!(?info, "Pointer scroll");

		panel_item.seat_data.pointer_event(
			&wl_surface,
			PointerEvent::Scroll {
				axis_continuous: info.axis_continuous,
				axis_discrete: info.axis_discrete,
			},
		);
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
		let toplevel = panel_item.toplevel_wl_surface();
		debug!(?toplevel, "Keyboard set keymap");

		let mut surfaces = vec![toplevel];
		surfaces.extend(panel_item.popups.lock().values().filter_map(|p| {
			let popup = p.upgrade().ok()?;
			let popup_data = PopupData::get(&popup).lock();
			let xdg_surface = popup_data.xdg_surface();
			let xdg_surface_data = XdgSurfaceData::get(&xdg_surface).lock();
			Some(xdg_surface_data.wl_surface())
		}));

		panel_item.seat_data.set_keymap(keymap, surfaces);

		Ok(())
	}

	fn keyboard_key_flex(node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		let Some(panel_item) = PanelItem::from_node(node) else { return Ok(()) };
		let (surface_id, key, state): (SurfaceID, u32, u32) = deserialize(data)?;
		let wl_surface = panel_item.wl_surface_from_id_result(&surface_id)?;
		debug!(key, state, "Set keyboard key state");

		panel_item
			.seat_data
			.keyboard_event(&wl_surface, KeyboardEvent::Key { key, state });

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
		let xdg_surface = panel_item.toplevel_xdg_surface();

		#[derive(Debug, Deserialize)]
		struct ConfigureToplevelInfo {
			size: Option<Vector2<u32>>,
			states: Vec<u32>,
			bounds: Option<Vector2<u32>>,
		}

		let info: ConfigureToplevelInfo = deserialize(data)?;
		debug!(info = ?&info, "Configure toplevel info");
		if let Some(bounds) = info.bounds {
			if xdg_toplevel.version() > EVT_CONFIGURE_BOUNDS_SINCE {
				xdg_toplevel.configure_bounds(bounds.x as i32, bounds.y as i32);
			}
		}
		let zero_size = Vector2::from([0; 2]);
		let size = info.size.unwrap_or(zero_size);
		// if size == zero_size && (info.states.contains(1) || info.states.contains(2)) {
		// 	xdg_toplevel.configure(
		// 		size.x as i32,
		// 		size.y as i32,
		// 		info.states
		// 			.into_iter()
		// 			.flat_map(|state| state.to_ne_bytes())
		// 			.collect(),
		// 	);
		// }
		xdg_toplevel.configure(
			size.x as i32,
			size.y as i32,
			info.states.into_iter().flat_map(u32::to_ne_bytes).collect(),
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
		if xdg_toplevel.version() < EVT_WM_CAPABILITIES_SINCE {
			return Ok(());
		}
		let xdg_surface = panel_item.toplevel_xdg_surface();

		let capabilities: Vec<u8> = deserialize(data)?;
		debug!("Set toplevel capabilities");
		xdg_toplevel.wm_capabilities(capabilities);
		xdg_surface.configure(SERIAL_COUNTER.inc());
		core_surface.flush_clients();

		Ok(())
	}

	pub fn commit_toplevel(&self) {
		// let mapped_size = self.core_surface().and_then(|c| c.size());
		let toplevel = self.toplevel();
		let state = ToplevelData::get(&toplevel);
		let state = state.lock();
		// let mut queued_state = state.queued_state.take().unwrap();
		// queued_state.mapped = mapped_size.is_some();
		// if let Some(size) = mapped_size {
		// 	queued_state.size = size;
		// 	queued_state.geometry.update_to_surface_size(size);
		// }
		// *state = (*queued_state).clone();
		// state.queued_state = Some(queued_state);

		debug!(state = ?&*state, "Commit toplevel");
		let Some(node) = self.node.upgrade() else { return };
		let _ = node.send_remote_signal("commit_toplevel", &serialize(&*state).unwrap());
	}

	pub fn recommend_toplevel_state(&self, state: RecommendedState) {
		let Some(node) = self.node.upgrade() else { return };
		let data = serialize(state).unwrap();
		debug!(?state, "Recommend toplevel state");

		let _ = node.send_remote_signal("recommend_toplevel_state", &data);
	}

	pub fn new_popup(&self, popup: &XdgPopup, data: &PopupData) {
		let uid = data.uid.clone();

		self.popups.lock().insert(uid.clone(), popup.downgrade());

		let Some(node) = self.node.upgrade() else { return };
		let _ = node.send_remote_signal("new_popup", &serialize(&(&uid, data)).unwrap());
	}
	// pub fn commit_popup(&self, data: &PopupData) {
	// 	let xdg_surf = data.xdg_surface.upgrade().unwrap();
	// 	let surf = xdg_surf
	// 		.data::<XdgSurfaceData>()
	// 		.unwrap()
	// 		.wl_surface
	// 		.upgrade()
	// 		.unwrap();

	// let core_surface =
	// 	compositor::with_states(&surf, |s| s.data_map.get::<Arc<CoreSurface>>().cloned())
	// 		.unwrap();
	// let mut popup_state = data.state.lock();
	// popup_state.mapped = core_surface.size().is_some();
	// }
	pub fn reposition_popup(&self, popup_state: &PopupData) {
		let Some(node) = self.node.upgrade() else { return };

		let _ = node.send_remote_signal(
			"reposition_popup",
			&serialize(popup_state.positioner_data().unwrap()).unwrap(),
		);
	}
	pub fn drop_popup(&self, uid: &str) {
		if let Some(popup) = self
			.popups
			.lock()
			.remove(uid)
			.and_then(|popup| popup.upgrade().ok()?.data::<Arc<PopupData>>().cloned())
		{
			self.seat_data.drop_surface(&popup.wl_surface());
		}

		let Some(node) = self.node.upgrade() else { return };
		let _ = node.send_remote_signal("drop_popup", &serialize(uid).unwrap());
	}

	pub fn grab_keyboard(&self, sid: Option<SurfaceID>) {
		let Some(node) = self.node.upgrade() else { return };

		let _ = node.send_remote_signal("grab_keyboard", &serialize(sid).unwrap());
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
		let toplevel = self.toplevel_wl_surface();
		self.seat_data.drop_surface(&toplevel);

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

		let toplevel = self.toplevel();
		let toplevel_state = ToplevelData::get(&toplevel);
		let toplevel_state = toplevel_state.lock().clone();

		let popups = self
			.popups
			.lock()
			.values()
			.filter_map(|v| Some(v.upgrade().ok()?.data::<Mutex<PopupData>>()?.lock().clone()))
			.collect::<Vec<_>>();

		let pointer_grab = self.pointer_grab.lock().clone();
		let keyboard_grab = self.keyboard_grab.lock().clone();

		serialize((
			id,
			(
				cursor_size.zip(cursor_hotspot),
				toplevel_state,
				popups,
				pointer_grab,
				keyboard_grab,
			),
		))
		.unwrap()
	}
}
impl Drop for PanelItem {
	fn drop(&mut self) {
		// Dropped panel item, basically just a debug breakpoint place
	}
}

use crate::{
	core::{
		client::{get_env, state, Client, INTERNAL_CLIENT},
		registry::Registry,
	},
	nodes::{
		drawable::model::ModelPart,
		items::{Item, ItemType, TypeInfo},
		spatial::Spatial,
		Message, Node,
	},
};
use color_eyre::eyre::{eyre, Result};
use glam::Mat4;
use lazy_static::lazy_static;
use mint::Vector2;
use nanoid::nanoid;
use rustc_hash::FxHashMap;
use serde::{
	de::{Deserializer, Error, SeqAccess, Visitor},
	ser::Serializer,
	Deserialize, Serialize,
};
use stardust_xr::schemas::flex::{deserialize, serialize};
use std::sync::{Arc, Weak};
use tracing::{debug, info};

lazy_static! {
	pub static ref ITEM_TYPE_INFO_PANEL: TypeInfo = TypeInfo {
		type_name: "panel",
		aliased_local_signals: vec![
			"apply_surface_material",
			"close_toplevel",
			"auto_size_toplevel",
			"set_toplevel_size",
			"set_toplevel_focused_visuals",
			"pointer_motion",
			"pointer_button",
			"pointer_scroll",
			"keyboard_keymap",
			"keyboard_key",
			"touch_down",
			"touch_move",
			"touch_up",
			"reset_touches",
		],
		aliased_local_methods: vec![],
		aliased_remote_signals: vec![
			"toplevel_parent_changed",
			"toplevel_title_changed",
			"toplevel_app_id_changed",
			"toplevel_fullscreen_active",
			"toplevel_move_request",
			"toplevel_resize_request",
			"toplevel_size_changed",
			"set_cursor",
			"new_child",
			"reposition_child",
			"drop_child",
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
	Child(String),
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
			"Child" => {
				let Some(text) = seq.next_element()? else {
					return Err(A::Error::missing_field("child_text"));
				};
				Ok(SurfaceID::Child(text))
			}
			_ => Err(A::Error::unknown_variant(
				discrim,
				&["Cursor", "Toplevel", "Child"],
			)),
		}
	}
}

impl serde::Serialize for SurfaceID {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		match self {
			Self::Cursor => ["Cursor"].serialize(serializer),
			Self::Toplevel => ["Toplevel"].serialize(serializer),
			Self::Child(text) => ["Child", text].serialize(serializer),
		}
	}
}

/// The origin and size of the surface's "solid" part.
#[derive(Debug, Serialize, Clone, Copy)]
pub struct Geometry {
	pub origin: Vector2<i32>,
	pub size: Vector2<u32>,
}
/// The state of the panel item's toplevel.
#[derive(Debug, Clone, Serialize)]
pub struct ToplevelInfo {
	/// The UID of the panel item of the parent of this toplevel, if it exists
	pub parent: Option<String>,
	/// Equivalent to the window title
	pub title: Option<String>,
	/// Application identifier, see <https://standards.freedesktop.org/desktop-entry-spec/>
	pub app_id: Option<String>,
	/// Current size in pixels
	pub size: Vector2<u32>,
	/// Recommended minimum size in pixels
	pub min_size: Option<Vector2<u32>>,
	/// Recommended maximum size in pixels
	pub max_size: Option<Vector2<u32>>,
	/// Surface geometry
	pub logical_rectangle: Geometry,
}

/// Data on positioning a child
#[derive(Debug, Clone, Serialize)]
pub struct ChildInfo {
	pub parent: SurfaceID,
	pub geometry: Geometry,
}

/// The init data for the panel item.
#[derive(Debug, Clone, Serialize)]
pub struct PanelItemInitData {
	/// The cursor, if applicable.
	pub cursor: Option<Geometry>,
	/// Size of the toplevel surface in pixels.
	pub toplevel: ToplevelInfo,
	/// Vector of childs that already exist
	pub children: FxHashMap<String, ChildInfo>,
	/// The surface, if any, that has exclusive input to the pointer.
	pub pointer_grab: Option<SurfaceID>,
	/// The surface, if any, that has exclusive input to the keyboard.
	pub keyboard_grab: Option<SurfaceID>,
}

pub trait Backend: Send + Sync + 'static {
	fn start_data(&self) -> Result<PanelItemInitData>;

	fn apply_surface_material(&self, surface: SurfaceID, model_part: &Arc<ModelPart>);

	fn close_toplevel(&self);
	fn auto_size_toplevel(&self);
	fn set_toplevel_size(&self, size: Vector2<u32>);
	fn set_toplevel_focused_visuals(&self, focused: bool);

	fn pointer_motion(&self, surface: &SurfaceID, position: Vector2<f32>);
	fn pointer_button(&self, surface: &SurfaceID, button: u32, pressed: bool);
	fn pointer_scroll(
		&self,
		surface: &SurfaceID,
		scroll_distance: Option<Vector2<f32>>,
		scroll_steps: Option<Vector2<f32>>,
	);

	fn keyboard_keys(&self, surface: &SurfaceID, keymap_id: &str, keys: Vec<i32>);

	fn touch_down(&self, surface: &SurfaceID, id: u32, position: Vector2<f32>);
	fn touch_move(&self, id: u32, position: Vector2<f32>);
	fn touch_up(&self, id: u32);
	fn reset_touches(&self);
}

pub fn panel_item_from_node(node: &Node) -> Option<Arc<dyn PanelItemTrait>> {
	let ItemType::Panel(panel_item) = &node.get_aspect::<Item>().ok()?.specialization else {
		return None;
	};
	Some(panel_item.clone())
}

pub trait PanelItemTrait: Backend + Send + Sync + 'static {
	fn uid(&self) -> &str;
	fn serialize_start_data(&self, id: &str) -> Result<Message>;
}

pub struct PanelItem<B: Backend + ?Sized> {
	pub uid: String,
	node: Weak<Node>,
	pub backend: Box<B>,
}
impl<B: Backend + ?Sized> PanelItem<B> {
	pub fn create(backend: Box<B>, pid: Option<i32>) -> (Arc<Node>, Arc<PanelItem<B>>) {
		debug!(?pid, "Create panel item");

		let startup_settings = pid
			.and_then(|pid| get_env(pid).ok())
			.and_then(|env| state(&env));

		let uid = nanoid!();
		let node = Arc::new(Node::create_parent_name(
			&INTERNAL_CLIENT,
			"/item/panel/item",
			&uid,
			true,
		));
		let spatial = Spatial::add_to(&node, None, Mat4::IDENTITY, false);
		if let Some(startup_settings) = &startup_settings {
			spatial.set_local_transform(startup_settings.root);
		}

		let panel_item = Arc::new(PanelItem {
			uid: uid.clone(),
			node: Arc::downgrade(&node),
			backend,
		});

		let generic_panel_item: Arc<dyn PanelItemTrait> = panel_item.clone();
		Item::add_to(
			&node,
			uid,
			&ITEM_TYPE_INFO_PANEL,
			ItemType::Panel(generic_panel_item),
		);

		node.add_local_signal("apply_surface_material", Self::apply_surface_material_flex);
		node.add_local_signal("close_toplevel", Self::close_toplevel_flex);
		node.add_local_signal("auto_size_toplevel", Self::auto_size_toplevel_flex);
		node.add_local_signal("set_toplevel_size", Self::set_toplevel_size_flex);

		node.add_local_signal("pointer_motion", Self::pointer_motion_flex);
		node.add_local_signal("pointer_button", Self::pointer_button_flex);
		node.add_local_signal("pointer_scroll", Self::pointer_scroll_flex);

		node.add_local_signal("keyboard_key", Self::keyboard_keys_flex);

		node.add_local_signal("touch_down", Self::touch_down_flex);
		node.add_local_signal("touch_move", Self::touch_move_flex);
		node.add_local_signal("touch_up", Self::touch_up_flex);
		node.add_local_signal("reset_touches", Self::reset_touches_flex);

		(node, panel_item)
	}
	pub fn drop_toplevel(&self) {
		let Some(node) = self.node.upgrade() else {
			return;
		};
		node.destroy();
	}
}

// Remote signals
impl<B: Backend + ?Sized> PanelItem<B> {
	pub fn toplevel_parent_changed(&self, parent: &str) {
		let Some(node) = self.node.upgrade() else {
			return;
		};
		let _ = node.send_remote_signal("toplevel_parent_changed", serialize(parent).unwrap());
	}
	pub fn toplevel_title_changed(&self, title: &str) {
		let Some(node) = self.node.upgrade() else {
			return;
		};
		let _ = node.send_remote_signal("toplevel_title_changed", serialize(title).unwrap());
	}
	pub fn toplevel_app_id_changed(&self, app_id: &str) {
		let Some(node) = self.node.upgrade() else {
			return;
		};
		let _ = node.send_remote_signal("toplevel_app_id_changed", serialize(app_id).unwrap());
	}
	pub fn toplevel_fullscreen_active(&self, active: bool) {
		let Some(node) = self.node.upgrade() else {
			return;
		};
		let _ = node.send_remote_signal("toplevel_fullscreen_active", serialize(active).unwrap());
	}
	pub fn toplevel_move_request(&self) {
		let Some(node) = self.node.upgrade() else {
			return;
		};
		let _ = node.send_remote_signal("toplevel_move_request", Vec::<u8>::new());
	}
	pub fn toplevel_resize_request(&self, up: bool, down: bool, left: bool, right: bool) {
		let Some(node) = self.node.upgrade() else {
			return;
		};
		let _ = node.send_remote_signal(
			"toplevel_resize_request",
			serialize((up, down, left, right)).unwrap(),
		);
	}
	pub fn toplevel_size_changed(&self, size: Vector2<u32>) {
		let Some(node) = self.node.upgrade() else {
			return;
		};
		let _ = node.send_remote_signal("toplevel_size_changed", serialize(size).unwrap());
	}

	pub fn set_cursor(&self, geometry: Option<Geometry>) {
		let Some(node) = self.node.upgrade() else {
			return;
		};
		let _ = node.send_remote_signal("set_cursor", serialize(geometry).unwrap());
	}

	pub fn new_child(&self, uid: &str, info: ChildInfo) {
		let Some(node) = self.node.upgrade() else {
			return;
		};
		let _ = node.send_remote_signal("new_child", serialize((uid, info)).unwrap());
	}
	pub fn reposition_child(&self, uid: &str, geometry: Geometry) {
		let Some(node) = self.node.upgrade() else {
			return;
		};
		let _ = node.send_remote_signal("reposition_child", serialize((uid, geometry)).unwrap());
	}
	pub fn drop_child(&self, uid: &str) {
		let Some(node) = self.node.upgrade() else {
			return;
		};
		let _ = node.send_remote_signal("drop_child", serialize(uid).unwrap());
	}
}
// Local signals
macro_rules! flex_no_args {
	($fn_name: ident, $trait_fn: ident) => {
		fn $fn_name(
			node: Arc<Node>,
			_calling_client: Arc<Client>,
			_message: Message,
		) -> Result<()> {
			let Some(panel_item) = panel_item_from_node(&node) else {
				return Ok(());
			};
			panel_item.$trait_fn();
			Ok(())
		}
	};
}
macro_rules! flex_deserialize {
	($fn_name: ident, $trait_fn: ident) => {
		fn $fn_name(node: Arc<Node>, _calling_client: Arc<Client>, message: Message) -> Result<()> {
			let Some(panel_item) = panel_item_from_node(&node) else {
				return Ok(());
			};
			panel_item.$trait_fn(deserialize(message.as_ref())?);
			Ok(())
		}
	};
}
impl<B: Backend + ?Sized> PanelItem<B> {
	fn apply_surface_material_flex(
		node: Arc<Node>,
		calling_client: Arc<Client>,
		message: Message,
	) -> Result<()> {
		let Some(panel_item) = panel_item_from_node(&node) else {
			return Ok(());
		};

		#[derive(Debug, Deserialize)]
		struct SurfaceMaterialInfo<'a> {
			surface: SurfaceID,
			model_node_path: &'a str,
		}

		let info: SurfaceMaterialInfo = deserialize(message.as_ref())?;

		let model_node = calling_client
			.scenegraph
			.get_node(info.model_node_path)
			.ok_or_else(|| eyre!("Model node not found"))?;
		let model_part = model_node.get_aspect::<ModelPart>()?;
		debug!(?info, "Apply surface material");

		panel_item.apply_surface_material(info.surface, &model_part);

		Ok(())
	}

	flex_no_args!(close_toplevel_flex, close_toplevel);
	flex_no_args!(auto_size_toplevel_flex, auto_size_toplevel);
	flex_deserialize!(set_toplevel_size_flex, set_toplevel_size);

	fn pointer_motion_flex(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		message: Message,
	) -> Result<()> {
		let Some(panel_item) = panel_item_from_node(&node) else {
			return Ok(());
		};

		let (surface_id, position): (SurfaceID, Vector2<f32>) = deserialize(message.as_ref())?;
		debug!(?surface_id, ?position, "Pointer deactivate");

		panel_item.pointer_motion(&surface_id, position);

		Ok(())
	}
	fn pointer_button_flex(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		message: Message,
	) -> Result<()> {
		let Some(panel_item) = panel_item_from_node(&node) else {
			return Ok(());
		};

		let (surface_id, button, state): (SurfaceID, u32, u32) = deserialize(message.as_ref())?;
		debug!(?surface_id, button, state, "Pointer button");

		panel_item.pointer_button(&surface_id, button, state != 0);
		Ok(())
	}
	fn pointer_scroll_flex(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		message: Message,
	) -> Result<()> {
		let Some(panel_item) = panel_item_from_node(&node) else {
			return Ok(());
		};

		#[derive(Debug, Deserialize)]
		struct PointerScrollInfo {
			surface_id: SurfaceID,
			axis_continuous: Option<Vector2<f32>>,
			axis_discrete: Option<Vector2<f32>>,
		}
		let info: PointerScrollInfo = deserialize(message.as_ref())?;
		debug!(?info, "Pointer scroll");

		panel_item.pointer_scroll(&info.surface_id, info.axis_continuous, info.axis_discrete);

		Ok(())
	}

	fn keyboard_keys_flex(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		message: Message,
	) -> Result<()> {
		let Some(panel_item) = panel_item_from_node(&node) else {
			return Ok(());
		};
		let (surface_id, keymap_id, keys): (SurfaceID, &str, Vec<i32>) =
			deserialize(message.as_ref())?;
		debug!(?keys, "Set keyboard key state");

		panel_item.keyboard_keys(&surface_id, keymap_id, keys);

		Ok(())
	}
	pub fn grab_keyboard(&self, sid: Option<SurfaceID>) {
		let Some(node) = self.node.upgrade() else {
			return;
		};

		let Ok(message) = serialize(sid) else { return };
		let _ = node.send_remote_signal("grab_keyboard", message);
	}

	fn touch_down_flex(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		message: Message,
	) -> Result<()> {
		let Some(panel_item) = panel_item_from_node(&node) else {
			return Ok(());
		};

		let (surface_id, id, position): (SurfaceID, u32, Vector2<f32>) =
			deserialize(message.as_ref())?;
		debug!(?surface_id, id, ?position, "Touch down");

		panel_item.touch_down(&surface_id, id, position);

		Ok(())
	}
	fn touch_move_flex(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		message: Message,
	) -> Result<()> {
		let Some(panel_item) = panel_item_from_node(&node) else {
			return Ok(());
		};

		let (id, position): (u32, Vector2<f32>) = deserialize(message.as_ref())?;
		debug!(?position, "Touch move");

		panel_item.touch_move(id, position);

		Ok(())
	}
	flex_deserialize!(touch_up_flex, touch_up);
	flex_no_args!(reset_touches_flex, reset_touches);
}
impl<B: Backend + ?Sized> PanelItemTrait for PanelItem<B> {
	fn uid(&self) -> &str {
		&self.uid
	}

	fn serialize_start_data(&self, id: &str) -> Result<Message> {
		Ok(serialize((id, self.start_data()?))?.into())
	}
}
impl<B: Backend + ?Sized> Backend for PanelItem<B> {
	fn start_data(&self) -> Result<PanelItemInitData> {
		self.backend.start_data()
	}

	fn apply_surface_material(&self, surface: SurfaceID, model_part: &Arc<ModelPart>) {
		self.backend.apply_surface_material(surface, model_part)
	}

	fn close_toplevel(&self) {
		self.backend.close_toplevel()
	}
	fn auto_size_toplevel(&self) {
		self.backend.auto_size_toplevel()
	}
	fn set_toplevel_size(&self, size: Vector2<u32>) {
		self.backend.set_toplevel_size(size)
	}
	fn set_toplevel_focused_visuals(&self, focused: bool) {
		self.backend.set_toplevel_focused_visuals(focused)
	}

	fn pointer_motion(&self, surface: &SurfaceID, position: Vector2<f32>) {
		self.backend.pointer_motion(surface, position)
	}
	fn pointer_button(&self, surface: &SurfaceID, button: u32, pressed: bool) {
		self.backend.pointer_button(surface, button, pressed)
	}
	fn pointer_scroll(
		&self,
		surface: &SurfaceID,
		scroll_distance: Option<Vector2<f32>>,
		scroll_steps: Option<Vector2<f32>>,
	) {
		self.backend
			.pointer_scroll(surface, scroll_distance, scroll_steps)
	}

	fn keyboard_keys(&self, surface: &SurfaceID, keymap_id: &str, keys: Vec<i32>) {
		self.backend.keyboard_keys(surface, keymap_id, keys)
	}

	fn touch_down(&self, surface: &SurfaceID, id: u32, position: Vector2<f32>) {
		self.backend.touch_down(surface, id, position)
	}
	fn touch_move(&self, id: u32, position: Vector2<f32>) {
		self.backend.touch_move(id, position)
	}
	fn touch_up(&self, id: u32) {
		self.backend.touch_up(id)
	}
	fn reset_touches(&self) {
		self.backend.reset_touches()
	}
}
impl<B: Backend + ?Sized> Drop for PanelItem<B> {
	fn drop(&mut self) {
		// Dropped panel item, basically just a debug breakpoint place
		info!("Dropped panel item {}", self.uid);
	}
}

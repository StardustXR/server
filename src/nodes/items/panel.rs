use crate::{
	core::{
		client::{get_env, startup_settings, Client, INTERNAL_CLIENT},
		registry::Registry,
	},
	nodes::{
		drawable::{model::ModelPart, Drawable},
		items::{self, Item, ItemType, TypeInfo},
		spatial::Spatial,
		Message, Node,
	},
};
use color_eyre::eyre::{bail, eyre, Result};
use glam::Mat4;
use lazy_static::lazy_static;
use mint::Vector2;
use nanoid::nanoid;
use serde::{
	de::{Deserializer, Error, SeqAccess, Visitor},
	ser::Serializer,
	Deserialize, Serialize,
};
use serde_repr::{Deserialize_repr, Serialize_repr};
use stardust_xr::schemas::flex::{deserialize, serialize};
use std::sync::{Arc, Weak};
use tracing::debug;

lazy_static! {
	pub static ref ITEM_TYPE_INFO_PANEL: TypeInfo = TypeInfo {
		type_name: "panel",
		aliased_local_signals: vec![
			"apply_surface_material",
			"close_toplevel",
			"auto_size_toplevel",
			"set_toplevel_size_changed",
			"set_toplevel_state",
			"set_toplevel_tiling",
			"set_toplevel_bounds",
			"set_maximize_enabled",
			"set_minimize_enabled",
			"set_fullscreen_enabled",
			"set_window_menu_enabled",
			"pointer_scroll",
			"pointer_button",
			"pointer_motion",
			"keyboard_key",
		],
		aliased_local_methods: vec![],
		aliased_remote_signals: vec![
			"toplevel_parent_changed",
			"toplevel_title_changed",
			"toplevel_app_id_changed",
			"toplevel_window_menu",
			"recommend_toplevel_state",
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

#[repr(u32)]
#[derive(Debug, Clone, Copy, Serialize_repr, Deserialize_repr)]
pub enum ToplevelState {
	Floating,
	Minimized,
	UnMinimized,
	Maximized,
	UnMaximized,
	Fullscreen,
	UnFullscreen,
}

/// Data on positioning a child
#[derive(Debug, Clone, Serialize)]
pub struct ChildInfo {
	pub uid: String,
	pub parent: SurfaceID,
	pub geometry: Geometry,
}

#[derive(Debug, Clone, Deserialize_repr)]
#[repr(u32)]
pub enum Edge {
	None = 0,
	Top = 1,
	Bottom = 2,
	Left = 4,
	TopLeft = 5,
	BottomLeft = 6,
	Right = 8,
	TopRight = 9,
	BottomRight = 10,
}

/// The init data for the panel item.
#[derive(Debug, Clone, Serialize)]
pub struct PanelItemInitData {
	/// The cursor, if applicable.
	pub cursor: Option<Geometry>,
	/// Size of the toplevel surface in pixels.
	pub toplevel: ToplevelInfo,
	/// Vector of childs that already exist
	pub children: Vec<ChildInfo>,
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
	fn toplevel_maximize(&self);
	fn toplevel_unmaximize(&self);
	fn toplevel_fullscreen(&self);
	fn toplevel_unfullscreen(&self);
	fn set_toplevel_tiling(&self, up: bool, down: bool, left: bool, right: bool);
	fn set_toplevel_bounds(&self, bounds: Option<Vector2<u32>>);
	fn set_toplevel_focused_visuals(&self, focused: bool);

	fn set_maximize_enabled(&self, enabled: bool);
	fn set_minimize_enabled(&self, enabled: bool);
	fn set_fullscreen_enabled(&self, enabled: bool);
	fn set_window_menu_enabled(&self, enabled: bool);

	fn pointer_motion(&self, surface: &SurfaceID, position: Vector2<f32>);
	fn pointer_button(&self, surface: &SurfaceID, button: u32, pressed: bool);
	fn pointer_scroll(
		&self,
		surface: &SurfaceID,
		scroll_distance: Option<Vector2<f32>>,
		scroll_steps: Option<Vector2<f32>>,
	);

	fn keyboard_key(&self, surface: &SurfaceID, key: u32, state: bool);
}

pub fn panel_item_from_node(node: &Node) -> Option<Arc<dyn PanelItemTrait>> {
	let ItemType::Panel(panel_item) = &node.item.get()?.specialization else {return None};
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
			.and_then(|env| startup_settings(&env));

		let uid = nanoid!();
		let node = Node::create(&INTERNAL_CLIENT, "/item/panel/item", &uid, true)
			.add_to_scenegraph()
			.unwrap();
		let spatial = Spatial::add_to(&node, None, Mat4::IDENTITY, false).unwrap();
		if let Some(startup_settings) = &startup_settings {
			spatial.set_local_transform(
				spatial.global_transform().inverse() * startup_settings.transform,
			);
		}

		let panel_item = Arc::new(PanelItem {
			uid: uid.clone(),
			node: Arc::downgrade(&node),
			backend,
		});

		let generic_panel_item: Arc<dyn PanelItemTrait> = panel_item.clone();
		let item = Item::add_to(
			&node,
			uid,
			&ITEM_TYPE_INFO_PANEL,
			ItemType::Panel(generic_panel_item),
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

		node.add_local_signal("apply_surface_material", Self::apply_surface_material_flex);
		node.add_local_signal("close_toplevel", Self::close_toplevel_flex);
		node.add_local_signal("auto_size_toplevel", Self::auto_size_toplevel_flex);
		node.add_local_signal("set_toplevel_size_changed", Self::set_toplevel_size_changed);
		node.add_local_signal("toplevel_maximize", Self::toplevel_maximize_flex);
		node.add_local_signal("toplevel_unmaximize", Self::toplevel_unmaximize_flex);
		node.add_local_signal("toplevel_fullscreen", Self::toplevel_fullscreen_flex);
		node.add_local_signal("toplevel_unfullscreen", Self::toplevel_unfullscreen_flex);
		node.add_local_signal("set_toplevel_tiling", Self::set_toplevel_tiling_flex);
		node.add_local_signal("set_toplevel_bounds", Self::set_toplevel_bounds_flex);

		node.add_local_signal("set_maximize_enabled", Self::set_maximize_enabled_flex);
		node.add_local_signal("set_minimize_enabled", Self::set_minimize_enabled_flex);
		node.add_local_signal("set_fullscreen_enabled", Self::set_fullscreen_enabled_flex);
		node.add_local_signal(
			"set_window_menu_enabled",
			Self::set_window_menu_enabled_flex,
		);

		node.add_local_signal("pointer_motion", Self::pointer_motion_flex);
		node.add_local_signal("pointer_button", Self::pointer_button_flex);
		node.add_local_signal("pointer_scroll", Self::pointer_scroll_flex);

		// node.add_local_signal(
		// 	"keyboard_set_keymap_string",
		// 	Self::keyboard_set_keymap_string_flex,
		// );
		// node.add_local_signal(
		// 	"keyboard_set_keymap_names",
		// 	Self::keyboard_set_keymap_names_flex,
		// );
		node.add_local_signal("keyboard_key", Self::keyboard_key_flex);

		(node, panel_item)
	}
	pub fn drop_toplevel(&self) {
		let Some(node) = self.node.upgrade() else {return};
		node.destroy();
	}
}

// Remote signals
impl<B: Backend + ?Sized> PanelItem<B> {
	pub fn toplevel_parent_changed(&self, parent: &str) {
		let Some(node) = self.node.upgrade() else {return};
		let _ = node.send_remote_signal("toplevel_parent_changed", serialize(parent).unwrap());
	}
	pub fn toplevel_title_changed(&self, title: &str) {
		let Some(node) = self.node.upgrade() else {return};
		let _ = node.send_remote_signal("toplevel_title_changed", serialize(title).unwrap());
	}
	pub fn toplevel_app_id_changed(&self, app_id: &str) {
		let Some(node) = self.node.upgrade() else {return};
		let _ = node.send_remote_signal("toplevel_app_id_changed", serialize(app_id).unwrap());
	}
	pub fn toplevel_window_menu(&self, offset: Vector2<i32>) {
		let Some(node) = self.node.upgrade() else {return};
		let _ = node.send_remote_signal("toplevel_window_menu", serialize(offset).unwrap());
	}
	pub fn recommend_toplevel_state(&self, state: ToplevelState) {
		let Some(node) = self.node.upgrade() else {return};
		let _ = node.send_remote_signal("recommend_toplevel_state", serialize(state).unwrap());
	}
	pub fn toplevel_move_request(&self) {
		let Some(node) = self.node.upgrade() else {return};
		let _ = node.send_remote_signal("toplevel_move_request", Vec::<u8>::new());
	}
	pub fn toplevel_resize_request(&self, up: bool, down: bool, left: bool, right: bool) {
		let Some(node) = self.node.upgrade() else {return};
		let _ = node.send_remote_signal(
			"toplevel_resize_request",
			serialize((up, down, left, right)).unwrap(),
		);
	}
	pub fn toplevel_size_changed(&self, size: Vector2<u32>) {
		let Some(node) = self.node.upgrade() else {return};
		let _ = node.send_remote_signal("toplevel_size_changed", serialize(size).unwrap());
	}

	pub fn set_cursor(&self, geometry: Option<Geometry>) {
		let Some(node) = self.node.upgrade() else {return};
		let _ = node.send_remote_signal("set_cursor", serialize(geometry).unwrap());
	}

	pub fn new_child(&self, info: ChildInfo) {
		let Some(node) = self.node.upgrade() else {return};
		let _ = node.send_remote_signal("new_child", serialize(info).unwrap());
	}
	pub fn reposition_child(&self, uid: &str, geometry: Geometry) {
		let Some(node) = self.node.upgrade() else {return};
		let _ = node.send_remote_signal("reposition_child", serialize((uid, geometry)).unwrap());
	}
	pub fn drop_child(&self, uid: &str) {
		let Some(node) = self.node.upgrade() else {return};
		let _ = node.send_remote_signal("drop_child", serialize(uid).unwrap());
	}
}
// Local signals
macro_rules! flex_no_args {
	($fn_name: ident, $trait_fn: ident) => {
		fn $fn_name(node: &Node, _calling_client: Arc<Client>, _message: Message) -> Result<()> {
			let Some(panel_item) = panel_item_from_node(node) else { return Ok(()) };
			panel_item.$trait_fn();
			Ok(())
		}
	};
}
macro_rules! flex_deserialize {
	($fn_name: ident, $trait_fn: ident) => {
		fn $fn_name(node: &Node, _calling_client: Arc<Client>, message: Message) -> Result<()> {
			let Some(panel_item) = panel_item_from_node(node) else { return Ok(()) };
			panel_item.$trait_fn(deserialize(message.as_ref())?);
			Ok(())
		}
	};
}
impl<B: Backend + ?Sized> PanelItem<B> {
	fn apply_surface_material_flex(
		node: &Node,
		calling_client: Arc<Client>,
		message: Message,
	) -> Result<()> {
		let Some(panel_item) = panel_item_from_node(node) else { return Ok(()) };

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
		let Some(Drawable::ModelPart(model_part)) = model_node.drawable.get() else {bail!("Node is not a model")};
		debug!(?info, "Apply surface material");

		panel_item.apply_surface_material(info.surface, model_part);

		Ok(())
	}

	flex_no_args!(close_toplevel_flex, close_toplevel);
	flex_no_args!(auto_size_toplevel_flex, auto_size_toplevel);
	flex_deserialize!(set_toplevel_size_changed, set_toplevel_size);
	flex_no_args!(toplevel_maximize_flex, toplevel_maximize);
	flex_no_args!(toplevel_unmaximize_flex, toplevel_unmaximize);
	flex_no_args!(toplevel_fullscreen_flex, toplevel_fullscreen);
	flex_no_args!(toplevel_unfullscreen_flex, toplevel_unfullscreen);
	flex_deserialize!(set_toplevel_bounds_flex, set_toplevel_bounds);
	fn set_toplevel_tiling_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		message: Message,
	) -> Result<()> {
		let Some(panel_item) = panel_item_from_node(node) else { return Ok(()) };
		let (up, down, left, right) = deserialize(message.as_ref())?;
		panel_item.set_toplevel_tiling(up, down, left, right);
		Ok(())
	}

	flex_deserialize!(set_maximize_enabled_flex, set_maximize_enabled);
	flex_deserialize!(set_minimize_enabled_flex, set_minimize_enabled);
	flex_deserialize!(set_fullscreen_enabled_flex, set_fullscreen_enabled);
	flex_deserialize!(set_window_menu_enabled_flex, set_window_menu_enabled);

	fn pointer_motion_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		message: Message,
	) -> Result<()> {
		let Some(panel_item) = panel_item_from_node(node) else { return Ok(()) };

		let (surface_id, position): (SurfaceID, Vector2<f32>) = deserialize(message.as_ref())?;
		debug!(?surface_id, ?position, "Pointer deactivate");

		panel_item.pointer_motion(&surface_id, position);

		Ok(())
	}
	fn pointer_button_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		message: Message,
	) -> Result<()> {
		let Some(panel_item) = panel_item_from_node(node) else { return Ok(()) };

		let (surface_id, button, state): (SurfaceID, u32, u32) = deserialize(message.as_ref())?;
		debug!(?surface_id, button, state, "Pointer button");

		panel_item.pointer_button(&surface_id, button, state != 0);
		Ok(())
	}
	fn pointer_scroll_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		message: Message,
	) -> Result<()> {
		let Some(panel_item) = panel_item_from_node(node) else { return Ok(()) };

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

	fn keyboard_key_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		message: Message,
	) -> Result<()> {
		let Some(panel_item) = panel_item_from_node(node) else { return Ok(()) };
		let (surface_id, key, state): (SurfaceID, u32, u32) = deserialize(message.as_ref())?;
		debug!(key, state, "Set keyboard key state");

		panel_item.keyboard_key(&surface_id, key, state == 0);

		Ok(())
	}
	pub fn grab_keyboard(&self, sid: Option<SurfaceID>) {
		let Some(node) = self.node.upgrade() else {return};

		let Ok(message) = serialize(sid) else {return};
		let _ = node.send_remote_signal("grab_keyboard", message);
	}
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

	fn set_toplevel_tiling(&self, up: bool, down: bool, left: bool, right: bool) {
		self.backend.set_toplevel_tiling(up, down, left, right)
	}
	fn set_toplevel_bounds(&self, bounds: Option<Vector2<u32>>) {
		self.backend.set_toplevel_bounds(bounds)
	}
	fn set_toplevel_focused_visuals(&self, focused: bool) {
		self.backend.set_toplevel_focused_visuals(focused)
	}
	fn toplevel_maximize(&self) {
		self.backend.toplevel_maximize()
	}
	fn toplevel_unmaximize(&self) {
		self.backend.toplevel_unmaximize()
	}
	fn toplevel_fullscreen(&self) {
		self.backend.toplevel_fullscreen()
	}
	fn toplevel_unfullscreen(&self) {
		self.backend.toplevel_unfullscreen()
	}
	fn set_maximize_enabled(&self, enabled: bool) {
		self.backend.set_maximize_enabled(enabled)
	}
	fn set_minimize_enabled(&self, enabled: bool) {
		self.backend.set_minimize_enabled(enabled)
	}
	fn set_fullscreen_enabled(&self, enabled: bool) {
		self.backend.set_fullscreen_enabled(enabled)
	}
	fn set_window_menu_enabled(&self, enabled: bool) {
		self.backend.set_window_menu_enabled(enabled)
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

	fn keyboard_key(&self, surface: &SurfaceID, key: u32, state: bool) {
		self.backend.keyboard_key(surface, key, state)
	}
}
impl<B: Backend + ?Sized> Drop for PanelItem<B> {
	fn drop(&mut self) {
		// Dropped panel item, basically just a debug breakpoint place
	}
}

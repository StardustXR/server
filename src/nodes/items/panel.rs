use super::camera::CameraItemAcceptor;
use super::{create_item_acceptor_flex, register_item_ui_flex};
use crate::bail;
use crate::core::Id;
use crate::nodes::{
	Aspect, AspectIdentifier,
	items::{ITEM_ACCEPTOR_ASPECT_ALIAS_INFO, ITEM_ASPECT_ALIAS_INFO, ITEM_UI_ASPECT_ALIAS_INFO},
};
use crate::{
	core::{
		client::{Client, INTERNAL_CLIENT, get_env, state},
		error::Result,
		registry::Registry,
	},
	nodes::{
		Node,
		drawable::model::ModelPart,
		items::{Item, ItemType, TypeInfo},
		spatial::{Spatial, Transform},
	},
};
use glam::Mat4;
use lazy_static::lazy_static;
use mint::Vector2;
use parking_lot::Mutex;
use slotmap::{DefaultKey, Key, KeyData, SlotMap};
use std::sync::{Arc, Weak};
use tracing::debug;

stardust_xr_server_codegen::codegen_item_panel_protocol!();
impl Default for Geometry {
	fn default() -> Self {
		Geometry {
			origin: [0, 0].into(),
			size: [0, 0].into(),
		}
	}
}
impl Copy for Geometry {}

lazy_static! {
	pub static ref KEYMAPS: Mutex<SlotMap<DefaultKey, String>> = Mutex::new(SlotMap::default());
	pub static ref ITEM_TYPE_INFO_PANEL: TypeInfo = TypeInfo {
		type_name: "panel",
		alias_info: PANEL_ITEM_ASPECT_ALIAS_INFO.clone(),
		ui_node_id: INTERFACE_NODE_ID,
		ui: Default::default(),
		items: Registry::new(),
		acceptors: Registry::new(),
		add_acceptor_aspect: |node| {
			node.add_aspect(PanelItemUi);
		},
		add_ui_aspect: |node| {
			node.add_aspect(PanelItemAcceptor);
		},
		new_acceptor_fn: |node, acceptor, acceptor_field| {
			let _ = panel_item_ui_client::create_acceptor(node, acceptor, acceptor_field);
		}
	};
}

pub trait Backend: Send + Sync + 'static {
	fn start_data(&self) -> Result<PanelItemInitData>;

	fn apply_cursor_material(&self, model_part: &Arc<ModelPart>);
	fn apply_surface_material(&self, surface: SurfaceId, model_part: &Arc<ModelPart>);

	fn close_toplevel(&self);
	fn auto_size_toplevel(&self);
	fn set_toplevel_size(&self, size: Vector2<u32>);
	fn set_toplevel_focused_visuals(&self, focused: bool);

	fn pointer_motion(&self, surface: &SurfaceId, position: Vector2<f32>);
	fn pointer_button(&self, surface: &SurfaceId, button: u32, pressed: bool);
	fn pointer_scroll(
		&self,
		surface: &SurfaceId,
		scroll_distance: Option<Vector2<f32>>,
		scroll_steps: Option<Vector2<f32>>,
	);

	fn keyboard_key(&self, surface: &SurfaceId, keymap_id: Id, key: u32, pressed: bool);

	fn touch_down(&self, surface: &SurfaceId, id: u32, position: Vector2<f32>);
	fn touch_move(&self, id: u32, position: Vector2<f32>);
	fn touch_up(&self, id: u32);
	fn reset_input(&self);
}

pub fn panel_item_from_node(node: &Node) -> Option<Arc<dyn PanelItemTrait>> {
	let ItemType::Panel(panel_item) = &node.get_aspect::<Item>().ok()?.specialization else {
		return None;
	};
	Some(panel_item.clone())
}

pub trait PanelItemTrait: Send + Sync + 'static {
	fn backend(&self) -> &dyn Backend;
	fn send_ui_item_created(&self, node: &Node, item: &Arc<Node>);
	fn send_acceptor_item_created(&self, node: &Node, item: &Arc<Node>);
}

#[derive(Debug)]
pub struct PanelItem<B: Backend> {
	pub node: Weak<Node>,
	pub backend: Box<B>,
}
impl<B: Backend> PanelItem<B> {
	#[cfg_attr(not(feature = "wayland"), allow(dead_code))]
	pub fn create(backend: Box<B>, pid: Option<i32>) -> (Arc<Node>, Arc<PanelItem<B>>) {
		debug!(?pid, "Create panel item");

		let startup_settings = pid
			.and_then(|pid| get_env(pid).ok())
			.and_then(|env| state(&env));

		let node = Arc::new(Node::generate(&INTERNAL_CLIENT, true));
		let spatial = Spatial::add_to(&node, None, Mat4::IDENTITY);
		if let Some(startup_settings) = &startup_settings {
			spatial.set_local_transform(startup_settings.root);
		}

		let panel_item = Arc::new(PanelItem {
			node: Arc::downgrade(&node),
			backend,
		});

		let generic_panel_item: Arc<dyn PanelItemTrait> = panel_item.clone();
		Item::add_to(
			&node,
			&ITEM_TYPE_INFO_PANEL,
			ItemType::Panel(generic_panel_item),
		);
		node.add_aspect_raw(panel_item.clone());

		(node, panel_item)
	}
}

// Remote signals
#[allow(unused)]
impl<B: Backend> PanelItem<B> {
	pub fn toplevel_parent_changed(&self, parent: Id) {
		let Some(node) = self.node.upgrade() else {
			return;
		};
		panel_item_client::toplevel_parent_changed(&node, parent);
	}
	pub fn toplevel_title_changed(&self, title: &str) {
		let Some(node) = self.node.upgrade() else {
			return;
		};
		panel_item_client::toplevel_title_changed(&node, title);
	}
	pub fn toplevel_app_id_changed(&self, app_id: &str) {
		let Some(node) = self.node.upgrade() else {
			return;
		};
		panel_item_client::toplevel_app_id_changed(&node, app_id);
	}
	pub fn toplevel_fullscreen_active(&self, active: bool) {
		let Some(node) = self.node.upgrade() else {
			return;
		};
		panel_item_client::toplevel_fullscreen_active(&node, active);
	}
	pub fn toplevel_move_request(&self) {
		let Some(node) = self.node.upgrade() else {
			return;
		};
		panel_item_client::toplevel_move_request(&node);
	}
	pub fn toplevel_resize_request(&self, up: bool, down: bool, left: bool, right: bool) {
		let Some(node) = self.node.upgrade() else {
			return;
		};
		panel_item_client::toplevel_resize_request(&node, up, down, left, right);
	}
	pub fn toplevel_size_changed(&self, size: Vector2<u32>) {
		let Some(node) = self.node.upgrade() else {
			return;
		};
		panel_item_client::toplevel_size_changed(&node, size);
	}

	pub fn set_cursor(&self, geometry: Option<Geometry>) {
		let Some(node) = self.node.upgrade() else {
			return;
		};
		if let Some(geometry) = geometry {
			panel_item_client::set_cursor(&node, &geometry);
		} else {
			panel_item_client::hide_cursor(&node);
		}
	}

	pub fn create_child(&self, id: Id, info: &ChildInfo) {
		let Some(node) = self.node.upgrade() else {
			return;
		};
		panel_item_client::create_child(&node, id, info);
	}
	pub fn reposition_child(&self, id: Id, geometry: &Geometry) {
		let Some(node) = self.node.upgrade() else {
			return;
		};
		panel_item_client::reposition_child(&node, id, geometry);
	}
	pub fn destroy_child(&self, id: Id) {
		let Some(node) = self.node.upgrade() else {
			return;
		};
		panel_item_client::destroy_child(&node, id);
	}
}
impl<B: Backend> AspectIdentifier for PanelItem<B> {
	impl_aspect_for_panel_item_aspect_id! {}
}
impl<B: Backend> Aspect for PanelItem<B> {
	impl_aspect_for_panel_item_aspect! {}
}
#[allow(unused)]
impl<B: Backend> PanelItemAspect for PanelItem<B> {
	#[doc = "Apply the cursor as a material to a model."]
	fn apply_cursor_material(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		model_part: Arc<Node>,
	) -> Result<()> {
		let Some(panel_item) = panel_item_from_node(&node) else {
			return Ok(());
		};
		let model_part = model_part.get_aspect::<ModelPart>()?;

		panel_item.backend().apply_cursor_material(&model_part);
		Ok(())
	}

	#[doc = "Apply a surface's visuals as a material to a model."]
	fn apply_surface_material(
		node: Arc<Node>,
		calling_client: Arc<Client>,
		surface: SurfaceId,
		model_part: Arc<Node>,
	) -> Result<()> {
		let Some(panel_item) = panel_item_from_node(&node) else {
			return Ok(());
		};
		let model_part = model_part.get_aspect::<ModelPart>()?;

		panel_item
			.backend()
			.apply_surface_material(surface, &model_part);
		Ok(())
	}

	#[doc = "Try to close the toplevel.\n        \n        The panel item UI handler or panel item acceptor will drop the panel item if this succeeds."]
	fn close_toplevel(node: Arc<Node>, _calling_client: Arc<Client>) -> Result<()> {
		let Some(panel_item) = panel_item_from_node(&node) else {
			return Ok(());
		};
		panel_item.backend().close_toplevel();
		Ok(())
	}

	#[doc = "Request a resize of the surface to whatever size the 2D app wants."]
	fn auto_size_toplevel(node: Arc<Node>, _calling_client: Arc<Client>) -> Result<()> {
		let Some(panel_item) = panel_item_from_node(&node) else {
			return Ok(());
		};
		panel_item.backend().auto_size_toplevel();
		Ok(())
	}

	#[doc = "Request a resize of the surface (in pixels)."]
	fn set_toplevel_size(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		size: mint::Vector2<u32>,
	) -> Result<()> {
		let Some(panel_item) = panel_item_from_node(&node) else {
			return Ok(());
		};
		panel_item.backend().set_toplevel_size(size);
		Ok(())
	}

	#[doc = "Tell the toplevel to appear focused visually if true, or unfocused if false."]
	fn set_toplevel_focused_visuals(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		focused: bool,
	) -> Result<()> {
		let Some(panel_item) = panel_item_from_node(&node) else {
			return Ok(());
		};
		panel_item.backend().set_toplevel_focused_visuals(focused);
		Ok(())
	}

	#[doc = "Send an event to set the pointer's position (in pixels, relative to top-left of surface). This will activate the pointer."]
	fn pointer_motion(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		surface: SurfaceId,
		position: mint::Vector2<f32>,
	) -> Result<()> {
		let Some(panel_item) = panel_item_from_node(&node) else {
			return Ok(());
		};
		panel_item.backend().pointer_motion(&surface, position);
		Ok(())
	}

	#[doc = "Send an event to set a pointer button's state if the pointer's active. The `button` is from the `input_event_codes` crate (e.g. BTN_LEFT for left click)."]
	fn pointer_button(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		surface: SurfaceId,
		button: u32,
		pressed: bool,
	) -> Result<()> {
		let Some(panel_item) = panel_item_from_node(&node) else {
			return Ok(());
		};
		panel_item
			.backend()
			.pointer_button(&surface, button, pressed);
		Ok(())
	}

	#[doc = "Send an event to scroll the pointer if it's active.\nScroll distance is a value in pixels corresponding to the `distance` the surface should be scrolled.\nScroll steps is a value in columns/rows corresponding to the wheel clicks of a mouse or such. This also supports fractions of a wheel click."]
	fn pointer_scroll(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		surface: SurfaceId,
		scroll_distance: mint::Vector2<f32>,
		scroll_steps: mint::Vector2<f32>,
	) -> Result<()> {
		let Some(panel_item) = panel_item_from_node(&node) else {
			return Ok(());
		};
		panel_item
			.backend()
			.pointer_scroll(&surface, Some(scroll_distance), Some(scroll_steps));
		Ok(())
	}

	#[doc = "Send an event to stop scrolling the pointer."]
	fn pointer_stop_scroll(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		surface: SurfaceId,
	) -> Result<()> {
		let Some(panel_item) = panel_item_from_node(&node) else {
			return Ok(());
		};
		panel_item.backend().pointer_scroll(&surface, None, None);
		Ok(())
	}

	#[doc = "Send a series of key presses and releases (positive keycode for pressed, negative for released)."]
	fn keyboard_key(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		surface: SurfaceId,
		keymap_id: Id,
		key: u32,
		pressed: bool,
	) -> Result<()> {
		let Some(panel_item) = panel_item_from_node(&node) else {
			return Ok(());
		};
		panel_item
			.backend()
			.keyboard_key(&surface, keymap_id, key, pressed);
		Ok(())
	}

	#[doc = "Put a touch down on this surface with the unique ID `uid` at `position` (in pixels) from top left corner of the surface."]
	fn touch_down(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		surface: SurfaceId,
		uid: u32,
		position: mint::Vector2<f32>,
	) -> Result<()> {
		let Some(panel_item) = panel_item_from_node(&node) else {
			return Ok(());
		};
		panel_item.backend().touch_down(&surface, uid, position);
		Ok(())
	}

	#[doc = "Move an existing touch point."]
	fn touch_move(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		uid: u32,
		position: mint::Vector2<f32>,
	) -> Result<()> {
		let Some(panel_item) = panel_item_from_node(&node) else {
			return Ok(());
		};
		panel_item.backend().touch_move(uid, position);
		Ok(())
	}

	#[doc = "Release a touch from its surface."]
	fn touch_up(node: Arc<Node>, _calling_client: Arc<Client>, uid: u32) -> Result<()> {
		let Some(panel_item) = panel_item_from_node(&node) else {
			return Ok(());
		};
		panel_item.backend().touch_up(uid);
		Ok(())
	}

	#[doc = "Reset all input, such as pressed keys and pointer clicks and touches. Useful for when it's newly captured into an item acceptor to make sure no input gets stuck."]
	fn reset_input(node: Arc<Node>, _calling_client: Arc<Client>) -> Result<()> {
		let Some(panel_item) = panel_item_from_node(&node) else {
			return Ok(());
		};
		panel_item.backend().reset_input();
		Ok(())
	}
}

pub struct PanelItemUi;
impl AspectIdentifier for PanelItemUi {
	impl_aspect_for_panel_item_ui_aspect_id! {}
}
impl Aspect for PanelItemUi {
	impl_aspect_for_panel_item_ui_aspect! {}
}
impl PanelItemUiAspect for PanelItemUi {}

pub struct PanelItemAcceptor;
impl AspectIdentifier for PanelItemAcceptor {
	impl_aspect_for_panel_item_acceptor_aspect_id! {}
}
impl Aspect for PanelItemAcceptor {
	impl_aspect_for_panel_item_acceptor_aspect! {}
}
impl PanelItemAcceptorAspect for PanelItemAcceptor {
	fn capture_item(node: Arc<Node>, _calling_client: Arc<Client>, item: Arc<Node>) -> Result<()> {
		super::acceptor_capture_item_flex(node, item)
	}
}

impl<B: Backend> PanelItemTrait for PanelItem<B> {
	fn backend(&self) -> &dyn Backend {
		self.backend.as_ref()
	}
	fn send_ui_item_created(&self, node: &Node, item: &Arc<Node>) {
		let Ok(init_data) = self.backend.start_data() else {
			return;
		};
		let _ = panel_item_ui_client::create_item(node, item, init_data);
	}
	fn send_acceptor_item_created(&self, node: &Node, item: &Arc<Node>) {
		let Ok(init_data) = self.backend.start_data() else {
			return;
		};
		let _ = panel_item_acceptor_client::capture_item(node, item, init_data);
	}
}

impl InterfaceAspect for Interface {
	#[doc = "Register this client to manage the items of a certain type and create default 3D UI for them."]
	fn register_panel_item_ui(node: Arc<Node>, calling_client: Arc<Client>) -> Result<()> {
		node.add_aspect(CameraItemAcceptor);
		register_item_ui_flex(calling_client, &ITEM_TYPE_INFO_PANEL)
	}

	#[doc = "Create an item acceptor to allow temporary ownership of a given type of item. Creates a node at `/item/<item_type>/acceptor/<name>`."]
	fn create_panel_item_acceptor(
		node: Arc<Node>,
		calling_client: Arc<Client>,
		id: Id,
		parent: Arc<Node>,
		transform: Transform,
		field: Arc<Node>,
	) -> Result<()> {
		node.add_aspect(PanelItemAcceptor);
		create_item_acceptor_flex(
			calling_client,
			id,
			parent,
			transform,
			&ITEM_TYPE_INFO_PANEL,
			field,
		)
	}

	async fn register_keymap(
		_node: Arc<Node>,
		_calling_client: Arc<Client>,
		keymap: String,
	) -> Result<Id> {
		let mut keymaps = KEYMAPS.lock();
		if let Some(found_keymap_id) = keymaps
			.iter()
			.filter(|(_k, v)| *v == &keymap)
			.map(|(k, _v)| k)
			.last()
		{
			return Ok(found_keymap_id.data().as_ffi().into());
		}

		let key = keymaps.insert(keymap);
		Ok(key.data().as_ffi().into())
	}

	async fn get_keymap(
		_node: Arc<Node>,
		_calling_client: Arc<Client>,
		keymap_id: Id,
	) -> Result<String> {
		let keymaps = KEYMAPS.lock();
		let Some(keymap) = keymaps.get(KeyData::from_ffi(keymap_id.0).into()) else {
			bail!("Could not find keymap. Try registering it");
		};

		Ok(keymap.clone())
	}
}

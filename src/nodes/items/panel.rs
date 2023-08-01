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
use stardust_xr::schemas::flex::{deserialize, serialize};
use std::sync::{Arc, Weak};
use tracing::debug;

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

pub trait Backend: Send + Sync + 'static {
	fn serialize_start_data(&self, id: &str) -> Result<Message>;
	fn serialize_toplevel(&self) -> Result<Message>;
	fn set_toplevel_capabilities(&self, capabilities: Vec<u8>);
	fn configure_toplevel(
		&self,
		size: Option<Vector2<u32>>,
		states: Vec<u32>,
		bounds: Option<Vector2<u32>>,
	);
	fn apply_surface_material(&self, surface: SurfaceID, model_part: &Arc<ModelPart>);

	fn pointer_motion(&self, surface: &SurfaceID, position: Vector2<f32>);
	fn pointer_button(&self, surface: &SurfaceID, button: u32, pressed: bool);
	fn pointer_scroll(
		&self,
		surface: &SurfaceID,
		scroll_distance: Option<Vector2<f32>>,
		scroll_steps: Option<Vector2<f32>>,
	);

	fn keyboard_set_keymap(&self, keymap: &str) -> Result<()>;
	fn keyboard_key(&self, surface: &SurfaceID, key: u32, state: bool);
}

pub fn panel_item_from_node(node: &Node) -> Option<Arc<dyn PanelItemTrait>> {
	let ItemType::Panel(panel_item) = &node.item.get()?.specialization else {return None};
	Some(panel_item.clone())
}

pub trait PanelItemTrait: Backend + Send + Sync + 'static {
	fn uid(&self) -> &str;
	// fn node(&self) -> Option<Arc<Node>>;
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

		// panel_item
		// 	.seat_data
		// 	.new_surface(&wl_surface, Arc::downgrade(&panel_item));

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
		node.add_local_signal("configure_toplevel", Self::configure_toplevel_flex);
		node.add_local_signal(
			"set_toplevel_capabilities",
			Self::set_toplevel_capabilities_flex,
		);
		node.add_local_signal("pointer_scroll", Self::pointer_scroll_flex);
		node.add_local_signal("pointer_button", Self::pointer_button_flex);
		node.add_local_signal("pointer_motion", Self::pointer_motion_flex);

		node.add_local_signal(
			"keyboard_set_keymap_string",
			Self::keyboard_set_keymap_string_flex,
		);
		// node.add_local_signal(
		// 	"keyboard_set_keymap_names",
		// 	Self::keyboard_set_keymap_names_flex,
		// );
		node.add_local_signal("keyboard_key", Self::keyboard_key_flex);

		(node, panel_item)
	}

	pub fn node(&self) -> Option<Arc<Node>> {
		self.node.upgrade()
	}

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

		panel_item.pointer_button(&surface_id, button, state == 0);
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

	fn keyboard_set_keymap_string_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		message: Message,
	) -> Result<()> {
		let keymap_string: &str = deserialize(message.as_ref())?;

		let Some(panel_item) = panel_item_from_node(node) else { return Ok(()) };
		debug!("Keyboard set keymap");
		panel_item.keyboard_set_keymap(keymap_string)

		// PanelItem::keyboard_set_keymap_flex(node, &keymap)
	}
	// fn keyboard_set_keymap_names_flex(
	// 	node: &Node,
	// 	_calling_client: Arc<Client>,
	// 	message: Message,
	// ) -> Result<()> {
	// 	#[derive(Debug, Deserialize)]
	// 	struct Names<'a> {
	// 		rules: &'a str,
	// 		model: &'a str,
	// 		layout: &'a str,
	// 		variant: &'a str,
	// 		options: Option<String>,
	// 	}
	// 	let names: Names = deserialize(message.as_ref())?;
	// 	let context = xkb::Context::new(0);
	// 	let keymap = Keymap::new_from_names(
	// 		&context,
	// 		names.rules,
	// 		names.model,
	// 		names.layout,
	// 		names.variant,
	// 		names.options,
	// 		XKB_KEYMAP_FORMAT_TEXT_V1,
	// 	)
	// 	.ok_or_else(|| eyre!("Keymap is not valid"))?;

	// 	PanelItem::keyboard_set_keymap_flex(node, &keymap)
	// }
	// fn keyboard_set_keymap_flex(node: &Node, keymap: &str) -> Result<()> {
	// 	let Some(panel_item): Option<Arc<PanelItem<dyn WaylandBackend>>> = panel_item_from_node(node) else { return Ok(()) };
	// 	debug!("Keyboard set keymap");

	// 	panel_item.seat_data.set_keymap(
	// 		keymap,
	// 		match &panel_item {
	// 			Backend::Wayland(w) => w.input_surfaces(),
	// 			#[cfg(feature = "xwayland")]
	// 			Backend::X11(_) => panel_item
	// 				.toplevel_wl_surface()
	// 				.map(|s| vec![s])
	// 				.unwrap_or_default(),
	// 		},
	// 	);

	// 	Ok(())
	// }
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

	fn configure_toplevel_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		message: Message,
	) -> Result<()> {
		let Some(panel_item) = panel_item_from_node(node) else { return Ok(()) };

		#[derive(Debug, Deserialize)]
		struct ConfigureToplevelInfo {
			size: Option<Vector2<u32>>,
			states: Vec<u32>,
			bounds: Option<Vector2<u32>>,
		}
		let info: ConfigureToplevelInfo = deserialize(message.as_ref())?;

		panel_item.configure_toplevel(info.size, info.states, info.bounds);
		Ok(())
	}

	fn set_toplevel_capabilities_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		message: Message,
	) -> Result<()> {
		let Some(panel_item) = panel_item_from_node(node) else { return Ok(()) };

		let capabilities: Vec<u8> = deserialize(message.as_ref())?;
		debug!("Set toplevel capabilities");
		panel_item.set_toplevel_capabilities(capabilities);

		Ok(())
	}

	pub fn commit_toplevel(&self) {
		debug!("Commit toplevel");
		let Some(node) = self.node.upgrade() else {return};
		let Ok(data) = self.backend.serialize_toplevel() else {return};
		let _ = node.send_remote_signal("commit_toplevel", data);
	}

	pub fn recommend_toplevel_state(&self, state: RecommendedState) {
		let Some(node) = self.node.upgrade() else {return};
		let data = serialize(state).unwrap();
		debug!(?state, "Recommend toplevel state");

		let _ = node.send_remote_signal("recommend_toplevel_state", data);
	}
}
impl<B: Backend + ?Sized> PanelItemTrait for PanelItem<B> {
	fn uid(&self) -> &str {
		&self.uid
	}
}
impl<B: Backend + ?Sized> Backend for PanelItem<B> {
	fn serialize_start_data(&self, id: &str) -> Result<Message> {
		self.backend.serialize_start_data(id)
	}

	fn serialize_toplevel(&self) -> Result<Message> {
		self.backend.serialize_toplevel()
	}

	fn set_toplevel_capabilities(&self, capabilities: Vec<u8>) {
		self.backend.set_toplevel_capabilities(capabilities)
	}

	fn configure_toplevel(
		&self,
		size: Option<Vector2<u32>>,
		states: Vec<u32>,
		bounds: Option<Vector2<u32>>,
	) {
		self.backend.configure_toplevel(size, states, bounds)
	}

	fn apply_surface_material(&self, surface: SurfaceID, model_part: &Arc<ModelPart>) {
		self.backend.apply_surface_material(surface, model_part)
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

	fn keyboard_set_keymap(&self, keymap: &str) -> Result<()> {
		self.backend.keyboard_set_keymap(keymap)
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

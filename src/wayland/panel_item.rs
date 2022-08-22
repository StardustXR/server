use crate::{
	core::{
		client::{Client, INTERNAL_CLIENT},
		registry::Registry,
	},
	nodes::{
		core::Node,
		item::{Item, ItemType, TypeInfo},
		spatial::Spatial,
	},
};
use anyhow::{anyhow, Result};
use glam::Mat4;
use lazy_static::lazy_static;
use nanoid::nanoid;
use smithay::{reexports::wayland_server::protocol::wl_surface::WlSurface, wayland::compositor};
use std::sync::Arc;

use super::surface::CoreSurface;

lazy_static! {
	static ref ITEM_TYPE_INFO_PANEL: TypeInfo = TypeInfo {
		type_name: "panel",
		aliased_local_signals: vec![
			"applySurfaceMaterial",
			"setPointerActive",
			"setPointerPosition",
			"setPointerButtonPressed",
			"scrollPointerAxis",
			"touchDown",
			"touchMove",
			"touchUp",
			"setKeyboardActive",
			"setKeymap",
			"setKeyState",
			"setKeyModStates",
			"setKeyRepeat",
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
	toplevel_surface: WlSurface,
}
impl PanelItem {
	pub fn create(toplevel_surface: WlSurface) -> Arc<Node> {
		let node = Node::create(&INTERNAL_CLIENT, "/item/panel/item", &nanoid!(), true)
			.add_to_scenegraph();
		Spatial::add_to(&node, None, Mat4::IDENTITY).unwrap();

		let specialization = ItemType::Panel(PanelItem { toplevel_surface });
		let item =
			ITEM_TYPE_INFO_PANEL
				.items
				.add(Item::new(&node, &ITEM_TYPE_INFO_PANEL, specialization));
		let _ = node.item.set(item);
		node.add_local_signal("applySurfaceMaterial", PanelItem::apply_surface_material);
		node
	}

	fn apply_surface_material(node: &Node, calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		let flex_vec = flexbuffers::Reader::get_root(data)?.get_vector()?;
		let material_idx = flex_vec.idx(0).get_u64()?;
		let model_node = calling_client
			.scenegraph
			.get_node(flex_vec.idx(0).as_str())
			.ok_or_else(|| anyhow!("Model node not found"))?;
		let model = model_node
			.model
			.get()
			.ok_or_else(|| anyhow!("Node is not a model"))?;

		if let ItemType::Panel(panel_item) = &node.item.get().unwrap().specialization {
			compositor::with_states(&panel_item.toplevel_surface, |states| {
				if let Some(core_surface) = states.data_map.get::<CoreSurface>() {
					if let Some(sk_mat) = core_surface.sk_mat.get() {
						model
							.pending_material_replacements
							.lock()
							.insert(material_idx as u32, sk_mat.clone());
					}
				}
			});
		}

		Ok(())
	}
}

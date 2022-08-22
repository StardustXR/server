use crate::{
	core::{client::INTERNAL_CLIENT, registry::Registry},
	nodes::{
		core::Node,
		item::{Item, ItemType, TypeInfo},
		spatial::Spatial,
	},
};
use glam::Mat4;
use lazy_static::lazy_static;
use nanoid::nanoid;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use std::sync::Arc;

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
		// node.add_local_method("getPath", PanelItem::get_path_flex);
		node
	}
}

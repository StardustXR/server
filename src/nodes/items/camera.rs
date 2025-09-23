#![allow(dead_code)]
use super::{Item, ItemType, create_item_acceptor_flex, register_item_ui_flex};
use crate::bail;
use crate::core::error::Result;
use crate::nodes::Aspect;
use crate::nodes::AspectIdentifier;
use crate::nodes::items::ITEM_ACCEPTOR_ASPECT_ALIAS_INFO;
use crate::nodes::items::ITEM_ASPECT_ALIAS_INFO;
use crate::{
	core::{client::Client, registry::Registry, scenegraph::MethodResponseSender},
	nodes::{
		Message, Node,
		drawable::model::ModelPart,
		items::TypeInfo,
		spatial::{Spatial, Transform},
	},
};
use glam::Mat4;
use lazy_static::lazy_static;
use mint::{ColumnMatrix4, Vector2};
use parking_lot::Mutex;

use stardust_xr::schemas::flex::{deserialize, serialize};
use std::sync::Arc;

stardust_xr_server_codegen::codegen_item_camera_protocol!();
lazy_static! {
	pub(super) static ref ITEM_TYPE_INFO_CAMERA: TypeInfo = TypeInfo {
		type_name: "camera",
		alias_info: CAMERA_ITEM_ASPECT_ALIAS_INFO.clone(),
		ui_node_id: INTERFACE_NODE_ID,
		ui: Default::default(),
		items: Registry::new(),
		acceptors: Registry::new(),
		add_acceptor_aspect: |node| {
			node.add_aspect(CameraItemAcceptor);
		},
		add_ui_aspect: |node| {
			node.add_aspect(CameraItemUi);
		},
		new_acceptor_fn: |node, acceptor, acceptor_field| {
			let _ = camera_item_ui_client::create_acceptor(node, acceptor, acceptor_field);
		}
	};
}

struct FrameInfo {
	proj_matrix: Mat4,
	px_size: Vector2<u32>,
}

pub struct CameraItem {
	space: Arc<Spatial>,
	frame_info: Mutex<FrameInfo>,
	applied_to: Registry<ModelPart>,
	apply_to: Registry<ModelPart>,
}
#[allow(unused)]
impl CameraItem {
	pub fn add_to(node: &Arc<Node>, proj_matrix: Mat4, px_size: Vector2<u32>) {
		let item = Arc::new(CameraItem {
			space: node.get_aspect::<Spatial>().unwrap().clone(),
			frame_info: Mutex::new(FrameInfo {
				proj_matrix,
				px_size,
			}),
			applied_to: Registry::new(),
			apply_to: Registry::new(),
		});
		Item::add_to(node, &ITEM_TYPE_INFO_CAMERA, ItemType::Camera(item.clone()));
		node.add_aspect_raw(item);
	}

	fn frame_flex(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		_message: Message,
		response: MethodResponseSender,
	) {
		response.wrap(move || {
			let ItemType::Camera(_camera) = &node.get_aspect::<Item>().unwrap().specialization
			else {
				bail!("Wrong item type?");
			};
			Ok(serialize(())?)
		});
	}

	fn apply_preview_material_flex(
		node: Arc<Node>,
		calling_client: Arc<Client>,
		message: Message,
	) -> Result<()> {
		let ItemType::Camera(camera) = &node.get_aspect::<Item>().unwrap().specialization else {
			bail!("Wrong item type?");
		};
		let model_part_node =
			calling_client.get_node("Model part", deserialize(&message.data).unwrap())?;
		let model_part = model_part_node.get_aspect::<ModelPart>()?;
		camera.applied_to.add_raw(&model_part);
		camera.apply_to.add_raw(&model_part);
		Ok(())
	}

	pub fn send_ui_item_created(&self, node: &Node, item: &Arc<Node>) {
		let _ = camera_item_ui_client::create_item(node, item);
	}
	pub fn send_acceptor_item_created(&self, node: &Node, item: &Arc<Node>) {
		let _ = camera_item_acceptor_client::capture_item(node, item);
	}
}
impl AspectIdentifier for CameraItem {
	impl_aspect_for_camera_item_aspect_id! {}
}
impl Aspect for CameraItem {
	impl_aspect_for_camera_item_aspect! {}
}
impl CameraItemAspect for CameraItem {}

pub struct CameraItemUi;
impl AspectIdentifier for CameraItemUi {
	impl_aspect_for_camera_item_ui_aspect_id! {}
}
impl Aspect for CameraItemUi {
	impl_aspect_for_camera_item_ui_aspect! {}
}
impl CameraItemUiAspect for CameraItemUi {}

pub struct CameraItemAcceptor;
impl AspectIdentifier for CameraItemAcceptor {
	impl_aspect_for_camera_item_acceptor_aspect_id! {}
}
impl Aspect for CameraItemAcceptor {
	impl_aspect_for_camera_item_acceptor_aspect! {}
}
impl CameraItemAcceptorAspect for CameraItemAcceptor {
	fn capture_item(node: Arc<Node>, _calling_client: Arc<Client>, item: Arc<Node>) -> Result<()> {
		super::acceptor_capture_item_flex(node, item)
	}
}

impl InterfaceAspect for Interface {
	#[doc = "Create a camera item at a specific location"]
	fn create_camera_item(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		id: u64,
		parent: Arc<Node>,
		transform: Transform,
		proj_matrix: ColumnMatrix4<f32>,
		px_size: Vector2<u32>,
	) -> Result<()> {
		let space = parent.get_aspect::<Spatial>()?;
		let transform = transform.to_mat4(true, true, false);

		let node = Node::from_id(&calling_client, id, false).add_to_scenegraph()?;
		Spatial::add_to(&node, None, transform * space.global_transform(), false);
		CameraItem::add_to(&node, proj_matrix.into(), px_size);
		Ok(())
	}

	#[doc = "Register this client to manage camera items and create default 3D UI for them."]
	fn register_camera_item_ui(node: Arc<Node>, calling_client: Arc<Client>) -> Result<()> {
		node.add_aspect(CameraItemUi);
		register_item_ui_flex(calling_client, &ITEM_TYPE_INFO_CAMERA)
	}

	#[doc = "Create an item acceptor to allow temporary ownership of a given type of item. Creates a node at `/item/camera/acceptor/<name>`."]
	fn create_camera_item_acceptor(
		node: Arc<Node>,
		calling_client: Arc<Client>,
		id: u64,
		parent: Arc<Node>,
		transform: Transform,
		field: Arc<Node>,
	) -> Result<()> {
		node.add_aspect(CameraItemAcceptor);
		create_item_acceptor_flex(
			calling_client,
			id,
			parent,
			transform,
			&ITEM_TYPE_INFO_CAMERA,
			field,
		)
	}
}

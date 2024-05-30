use super::{create_item_acceptor_flex, register_item_ui_flex, Item, ItemInterface, ItemType};
use crate::{
	core::{
		client::{Client, INTERNAL_CLIENT},
		registry::Registry,
		scenegraph::MethodResponseSender,
	},
	create_interface,
	nodes::{
		drawable::{model::ModelPart, shaders::UNLIT_SHADER_BYTES},
		items::TypeInfo,
		spatial::{Spatial, Transform},
		Message, Node,
	},
};
use color_eyre::eyre::{bail, eyre, Result};
use glam::Mat4;
use lazy_static::lazy_static;
use mint::{ColumnMatrix4, Vector2};
use nanoid::nanoid;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use send_wrapper::SendWrapper;
use stardust_xr::schemas::flex::{deserialize, serialize};
use std::sync::Arc;
use stereokit_rust::{
	material::{Material, Transparency},
	shader::Shader,
	sk::MainThreadToken,
	system::Renderer,
	tex::{Tex, TexFormat, TexType},
	util::Color128,
};
use tracing::error;

stardust_xr_server_codegen::codegen_item_camera_protocol!();
lazy_static! {
	pub(super) static ref ITEM_TYPE_INFO_CAMERA: TypeInfo = TypeInfo {
		type_name: "camera",
		aliased_local_signals: vec!["apply_preview_material", "frame"],
		aliased_local_methods: vec![],
		aliased_remote_signals: vec![],
		ui: Default::default(),
		items: Registry::new(),
		acceptors: Registry::new(),
		new_acceptor_fn: |node, uid, acceptor, acceptor_field| {
			let _ = camera_item_ui_client::create_acceptor(node, uid, acceptor, acceptor_field);
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
	sk_tex: OnceCell<SendWrapper<Tex>>,
	sk_mat: OnceCell<Arc<SendWrapper<Material>>>,
	applied_to: Registry<ModelPart>,
	apply_to: Registry<ModelPart>,
}
impl CameraItem {
	pub fn add_to(node: &Arc<Node>, proj_matrix: Mat4, px_size: Vector2<u32>) {
		Item::add_to(
			node,
			nanoid!(),
			&ITEM_TYPE_INFO_CAMERA,
			ItemType::Camera(CameraItem {
				space: node.get_aspect::<Spatial>().unwrap().clone(),
				frame_info: Mutex::new(FrameInfo {
					proj_matrix,
					px_size,
				}),
				sk_tex: OnceCell::new(),
				sk_mat: OnceCell::new(),
				applied_to: Registry::new(),
				apply_to: Registry::new(),
			}),
		);
		node.add_local_method("frame", CameraItem::frame_flex);
		node.add_local_signal(
			"apply_preview_material",
			CameraItem::apply_preview_material_flex,
		);
	}

	fn frame_flex(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		_message: Message,
		response: MethodResponseSender,
	) {
		response.wrap_sync(move || {
			let ItemType::Camera(_camera) = &node.get_aspect::<Item>().unwrap().specialization
			else {
				return Err(eyre!("Wrong item type?"));
			};
			Ok(serialize(())?.into())
		});
	}

	fn apply_preview_material_flex(
		node: Arc<Node>,
		calling_client: Arc<Client>,
		message: Message,
	) -> Result<()> {
		let ItemType::Camera(camera) = &node.get_aspect::<Item>().unwrap().specialization else {
			bail!("Wrong item type?")
		};
		let model_part_node =
			calling_client.get_node("Model part", deserialize(&message.data).unwrap())?;
		let model_part = model_part_node.get_aspect::<ModelPart>()?;
		camera.applied_to.add_raw(&model_part);
		camera.apply_to.add_raw(&model_part);
		Ok(())
	}

	pub fn send_ui_item_created(&self, node: &Node, uid: &str, item: &Arc<Node>) {
		let _ = camera_item_ui_client::create_item(node, uid, item);
	}
	pub fn send_acceptor_item_created(&self, node: &Node, uid: &str, item: &Arc<Node>) {
		let _ = camera_item_acceptor_client::capture_item(node, uid, item);
	}

	pub fn update(&self, token: &MainThreadToken) {
		let frame_info = self.frame_info.lock();
		let sk_tex = self.sk_tex.get_or_init(|| {
			SendWrapper::new(Tex::gen_color(
				Color128::default(),
				frame_info.px_size.x as i32,
				frame_info.px_size.y as i32,
				TexType::Rendertarget,
				TexFormat::RGBA32Linear,
			))
		});
		let sk_mat = self
			.sk_mat
			.get_or_try_init(|| -> Result<Arc<SendWrapper<Material>>> {
				let shader = Shader::from_memory(&UNLIT_SHADER_BYTES)?;
				let mut mat = Material::new(&shader, None);
				mat.get_all_param_info().set_texture("diffuse", &**sk_tex);
				mat.transparency(Transparency::Blend);
				Ok(Arc::new(SendWrapper::new(mat)))
			});
		let Ok(sk_mat) = sk_mat else {
			error!("unable to make camera item stereokit texture");
			return;
		};
		for model_part in self.apply_to.take_valid_contents() {
			model_part.replace_material(sk_mat.clone())
		}

		if !self.applied_to.is_empty() {
			Renderer::render_to(
				token,
				&**sk_tex,
				self.space.global_transform(),
				frame_info.proj_matrix,
				None,
				None,
				None,
			)
		}
	}
}

pub fn update(token: &MainThreadToken) {
	for camera in ITEM_TYPE_INFO_CAMERA.items.get_valid_contents() {
		let ItemType::Camera(camera) = &camera.specialization else {
			continue;
		};
		camera.update(token);
	}
}

create_interface!(ItemInterface, ItemCameraInterfaceAspect, "/item/camera");
impl ItemCameraInterfaceAspect for ItemInterface {
	#[doc = "Create a camera item at a specific location"]
	fn create_camera_item(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		name: String,
		parent: Arc<Node>,
		transform: Transform,
		proj_matrix: ColumnMatrix4<f32>,
		px_size: Vector2<u32>,
	) -> Result<()> {
		let parent_name = format!("/item/{}/item", ITEM_TYPE_INFO_CAMERA.type_name);
		let space = parent.get_aspect::<Spatial>()?;
		let transform = transform.to_mat4(true, true, false);

		let node = Node::create_parent_name(&INTERNAL_CLIENT, &parent_name, &name, false)
			.add_to_scenegraph()?;
		Spatial::add_to(&node, None, transform * space.global_transform(), false);
		CameraItem::add_to(&node, proj_matrix.into(), px_size);
		node.get_aspect::<Item>().unwrap().make_alias_named(
			&calling_client,
			&parent_name,
			&name,
		)?;
		Ok(())
	}

	#[doc = "Register this client to manage camera items and create default 3D UI for them."]
	fn register_camera_item_ui(_node: Arc<Node>, calling_client: Arc<Client>) -> Result<()> {
		register_item_ui_flex(calling_client, &ITEM_TYPE_INFO_CAMERA)
	}

	#[doc = "Create an item acceptor to allow temporary ownership of a given type of item. Creates a node at `/item/camera/acceptor/<name>`."]
	fn create_camera_item_acceptor(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		name: String,
		parent: Arc<Node>,
		transform: Transform,
		field: Arc<Node>,
	) -> Result<()> {
		create_item_acceptor_flex(
			calling_client,
			name,
			parent,
			transform,
			&ITEM_TYPE_INFO_CAMERA,
			field,
		)
	}
}

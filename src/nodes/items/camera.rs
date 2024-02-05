use super::{Item, ItemType};
use crate::{
	core::{
		client::{Client, INTERNAL_CLIENT},
		registry::Registry,
		scenegraph::MethodResponseSender,
	},
	nodes::{
		drawable::{model::ModelPart, shaders::UNLIT_SHADER_BYTES},
		items::TypeInfo,
		spatial::{parse_transform, Spatial, Transform},
		Message, Node,
	},
};
use color_eyre::eyre::{bail, eyre, Result};
use glam::Mat4;
use lazy_static::lazy_static;
use mint::{RowMatrix4, Vector2};
use nanoid::nanoid;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use serde::Deserialize;
use stardust_xr::schemas::flex::{deserialize, serialize};
use std::sync::Arc;
use stereokit::{
	Color128, Material, Rect, RenderLayer, StereoKitDraw, Tex, TextureType, Transparency,
};

lazy_static! {
	pub(super) static ref ITEM_TYPE_INFO_CAMERA: TypeInfo = TypeInfo {
		type_name: "camera",
		aliased_local_signals: vec!["apply_preview_material", "frame"],
		aliased_local_methods: vec![],
		aliased_remote_signals: vec![],
		ui: Default::default(),
		items: Registry::new(),
		acceptors: Registry::new(),
	};
}

struct FrameInfo {
	proj_matrix: Mat4,
	px_size: Vector2<u32>,
}

pub struct CameraItem {
	space: Arc<Spatial>,
	frame_info: Mutex<FrameInfo>,
	sk_tex: OnceCell<Tex>,
	sk_mat: OnceCell<Arc<Material>>,
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

	pub fn serialize_start_data(&self, id: &str) -> Result<Message> {
		Ok(serialize(id)?.into())
	}

	pub fn update(&self, sk: &impl StereoKitDraw) {
		let frame_info = self.frame_info.lock();
		let sk_tex = self.sk_tex.get_or_init(|| {
			sk.tex_gen_color(
				Color128::default(),
				frame_info.px_size.x as i32,
				frame_info.px_size.y as i32,
				TextureType::RENDER_TARGET,
				stereokit::TextureFormat::RGBA32Linear,
			)
		});
		let sk_mat = self.sk_mat.get_or_init(|| {
			let shader = sk.shader_create_mem(&UNLIT_SHADER_BYTES).unwrap();
			let mat = sk.material_create(&shader);
			sk.material_set_texture(&mat, "diffuse", sk_tex.as_ref());
			sk.material_set_transparency(&mat, Transparency::Blend);
			Arc::new(mat)
		});
		for model_part in self.apply_to.take_valid_contents() {
			model_part.replace_material(sk_mat.clone())
		}

		if !self.applied_to.is_empty() {
			sk.render_to(
				sk_tex,
				frame_info.proj_matrix,
				self.space.global_transform(),
				RenderLayer::all(),
				stereokit::RenderClear::All,
				Rect {
					x: 0.0,
					y: 0.0,
					w: 0.0,
					h: 0.0,
				},
			);
		}
	}
}

pub fn update(sk: &impl StereoKitDraw) {
	for camera in ITEM_TYPE_INFO_CAMERA.items.get_valid_contents() {
		let ItemType::Camera(camera) = &camera.specialization else {
			continue;
		};
		camera.update(sk);
	}
}

pub(super) fn create_camera_item_flex(
	_node: Arc<Node>,
	calling_client: Arc<Client>,
	message: Message,
) -> Result<()> {
	#[derive(Deserialize)]
	struct CreateCameraItemInfo<'a> {
		name: &'a str,
		parent_path: &'a str,
		transform: Transform,
		proj_matrix: RowMatrix4<f32>,
		px_size: Vector2<u32>,
	}
	let info: CreateCameraItemInfo = deserialize(message.as_ref())?;
	let parent_name = format!("/item/{}/item", ITEM_TYPE_INFO_CAMERA.type_name);
	let space = calling_client
		.get_node("Spatial parent", info.parent_path)?
		.get_aspect::<Spatial>()?;
	let transform = parse_transform(info.transform, true, true, false);

	let node = Node::create_parent_name(&INTERNAL_CLIENT, &parent_name, info.name, false)
		.add_to_scenegraph()?;
	Spatial::add_to(&node, None, transform * space.global_transform(), false);
	CameraItem::add_to(&node, info.proj_matrix.into(), info.px_size);
	node.get_aspect::<Item>().unwrap().make_alias_named(
		&calling_client,
		&parent_name,
		info.name,
	)?;
	Ok(())
}

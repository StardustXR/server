use super::{Item, ItemType};
use crate::{
	core::{
		client::{Client, INTERNAL_CLIENT},
		registry::Registry,
		scenegraph::MethodResponseSender,
	},
	nodes::{
		drawable::{model::ModelPart, shaders::UNLIT_SHADER_BYTES, Drawable},
		items::TypeInfo,
		spatial::{find_spatial_parent, parse_transform, Spatial},
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
use rustc_hash::FxHashMap;
use serde::Deserialize;
use stardust_xr::{
	scenegraph::ScenegraphError,
	schemas::flex::{deserialize, serialize},
	values::Transform,
};
use std::sync::Arc;
use stereokit::{
	Color128, Material, Rect, RenderLayer, StereoKitDraw, Tex, TextureType, Transparency,
};
use tokio::sync::{mpsc, oneshot};

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
	sk_textures: Mutex<FxHashMap<(usize, usize), Tex>>,
	sk_mat: OnceCell<Arc<Material>>,
	render_requests_tx: mpsc::Sender<(usize, oneshot::Sender<()>)>,
	render_requests_rx: mpsc::Receiver<(usize, oneshot::Sender<()>)>,
	rendered_notifiers: Mutex<Vec<oneshot::Sender<()>>>,
	applied_to: Registry<ModelPart>,
	apply_to: Registry<ModelPart>,
}
impl CameraItem {
	pub fn add_to(node: &Arc<Node>, proj_matrix: Mat4, px_size: Vector2<u32>) {
		let (render_requests_tx, render_requests_rx) = mpsc::channel(5);
		Item::add_to(
			node,
			nanoid!(),
			&ITEM_TYPE_INFO_CAMERA,
			ItemType::Camera(CameraItem {
				space: node.spatial.get().unwrap().clone(),
				frame_info: Mutex::new(FrameInfo {
					proj_matrix,
					px_size,
				}),
				sk_textures: Mutex::new(FxHashMap::default()),
				sk_mat: OnceCell::new(),
				render_requests_tx,
				render_requests_rx,
				rendered_notifiers: Mutex::new(Vec::new()),
				applied_to: Registry::new(),
				apply_to: Registry::new(),
			}),
		);
		node.add_local_method("render", CameraItem::render_flex);
		node.add_local_signal(
			"apply_preview_material",
			CameraItem::apply_preview_material_flex,
		);
	}

	fn render_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		message: Message,
		response: MethodResponseSender,
	) {
		let ItemType::Camera(camera) = &node.item.get().unwrap().specialization else {
			let _ = response.send(Err(ScenegraphError::MethodError {
				error: "Wrong item type?".to_string(),
			}));
			return;
		};

		let (rendered_tx, rendered_rx) = oneshot::channel();

		let buffer_to_render: usize = deserialize(&message.data).unwrap();

		tokio::task::spawn(async move {
			camera
				.render_requests_tx
				.send((buffer_to_render, rendered_tx))
				.await;

			rendered_rx.await;
			response.send(Ok(Vec::new().into()));
		});
	}

	fn apply_preview_material_flex(
		node: &Node,
		calling_client: Arc<Client>,
		message: Message,
	) -> Result<()> {
		let ItemType::Camera(camera) = &node.item.get().unwrap().specialization else {
			bail!("Wrong item type?")
		};

		let model_part_node =
			calling_client.get_node("Model part", deserialize(&message.data).unwrap())?;
		let Drawable::ModelPart(model_part) =
			model_part_node.get_aspect("Model part", "model part", |n| &n.drawable)?
		else {
			bail!("Drawable is not a model node")
		};

		camera.applied_to.add_raw(model_part);
		camera.apply_to.add_raw(model_part);

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

		//if !self.applied_to.is_empty() {
		//}

		let mut render_notifiers = self.rendered_notifiers.lock();
		while let Ok((buffer_to_render, rendered_tx)) = self.render_requests_rx.try_recv() {
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
			render_notifiers.push(rendered_tx);
		}
	}

	pub fn send_rendered(&self) {
		for notifier in self.rendered_notifiers.lock().drain(..) {
			notifier.send(());
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

pub fn send_rendered() {
	for camera in ITEM_TYPE_INFO_CAMERA.items.get_valid_contents() {
		let ItemType::Camera(camera) = &camera.specialization else {
			continue;
		};
		camera.send_rendered();
	}
}

pub(super) fn create_camera_item_flex(
	_node: &Node,
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
	let space = find_spatial_parent(&calling_client, info.parent_path)?;
	let transform = parse_transform(info.transform, true, true, false);

	let node =
		Node::create(&INTERNAL_CLIENT, &parent_name, info.name, false).add_to_scenegraph()?;
	Spatial::add_to(&node, None, transform * space.global_transform(), false)?;
	CameraItem::add_to(&node, info.proj_matrix.into(), info.px_size);
	node.item
		.get()
		.unwrap()
		.make_alias_named(&calling_client, &parent_name, info.name)?;
	Ok(())
}

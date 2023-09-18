use super::{Item, ItemType};
use crate::{
	core::{
		buffers::BufferManager,
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
use color_eyre::eyre::{bail, Result};
use glam::Mat4;
use lazy_static::lazy_static;
use mint::{RowMatrix4, Vector2};
use nanoid::nanoid;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use serde::Deserialize;
use smithay::{
	backend::{
		allocator::{
			dmabuf::{Dmabuf, DmabufFlags},
			Buffer,
		},
		renderer::ImportDma,
	},
	utils::Size,
};
use stardust_xr::{
	scenegraph::ScenegraphError,
	schemas::flex::{deserialize, serialize},
	values::{BufferInfo, Transform},
};
use std::{ffi::c_void, sync::Arc};
use stereokit::{
	Color128, Material, Rect, RenderLayer, StereoKitDraw, Tex, TextureFormat, TextureType,
	Transparency,
};
use tokio::sync::mpsc;

lazy_static! {
	pub(super) static ref ITEM_TYPE_INFO_CAMERA: TypeInfo = TypeInfo {
		type_name: "camera",
		aliased_local_signals: vec!["apply_preview_material"],
		aliased_local_methods: vec!["render"],
		aliased_remote_signals: vec![],
		ui: Default::default(),
		items: Registry::new(),
		acceptors: Registry::new(),
	};
}

struct FrameInfo {
	proj_matrix: Mat4,
	preview_size: Vector2<u32>,
}

pub struct CameraItem {
	space: Arc<Spatial>,
	frame_info: Mutex<FrameInfo>,
	sk_tex: OnceCell<Tex>,
	sk_mat: OnceCell<Arc<Material>>,
	render_requests_tx: mpsc::Sender<(Dmabuf, MethodResponseSender)>,
	render_requests_rx: Mutex<mpsc::Receiver<(Dmabuf, MethodResponseSender)>>,
	rendered_notifiers: Mutex<Vec<MethodResponseSender>>,
	applied_preview_to: Registry<ModelPart>,
	apply_preview_to: Registry<ModelPart>,
}
impl CameraItem {
	pub fn add_to(node: &Arc<Node>, proj_matrix: Mat4, preview_size: Vector2<u32>) {
		let (render_requests_tx, render_requests_rx) = mpsc::channel(5);
		let camera_specialization = CameraItem {
			space: node.spatial.get().unwrap().clone(),
			frame_info: Mutex::new(FrameInfo {
				proj_matrix,
				preview_size,
			}),
			sk_tex: OnceCell::new(),
			sk_mat: OnceCell::new(),
			render_requests_tx,
			render_requests_rx: Mutex::new(render_requests_rx),
			rendered_notifiers: Mutex::new(Vec::new()),
			applied_preview_to: Registry::new(),
			apply_preview_to: Registry::new(),
		};
		Item::add_to(
			node,
			nanoid!(),
			&ITEM_TYPE_INFO_CAMERA,
			ItemType::Camera(camera_specialization),
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
		let Some(item) = node.item.get() else {
			let _ = response.send(Err(ScenegraphError::MethodError { error: "Item not found".to_string() }));
			return
		};
		let ItemType::Camera(camera) = &item.specialization else {
			let _ = response.send(Err(ScenegraphError::MethodError {
				error: "Wrong item type?".to_string(),
			}));
			return
		};

		let buffer_info: BufferInfo = deserialize(&message.data).unwrap();
		let mut fds = message.fds.iter();
		let mut builder = Dmabuf::builder(
			Into::<Size<i32, smithay::utils::Buffer>>::into((
				buffer_info.size.0 as i32,
				buffer_info.size.1 as i32,
			)),
			buffer_info.fourcc,
			DmabufFlags::from_bits_truncate(buffer_info.flags),
		);
		for plane in buffer_info.planes {
			builder.add_plane(
				fds.next().unwrap().try_clone().unwrap(),
				plane.idx,
				plane.offset,
				plane.stride,
				plane.modifier,
			);
		}
		let buffer_to_render = builder.build().unwrap();

		let _ = camera
			.render_requests_tx
			.try_send((buffer_to_render, response));
		// tokio::task::spawn(async move {
		// 	let result = rendered_rx.await;
		// 	response.wrap_sync(|| {
		// 		let result = result.map_err(|_| eyre!("failed to recieve response"))?;
		// 		result.map_err(|e| eyre!(e))?;
		// 		Ok(Message::from(Vec::new()))
		// 	});
		// });
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

		camera.applied_preview_to.add_raw(model_part);
		camera.apply_preview_to.add_raw(model_part);

		Ok(())
	}

	pub fn serialize_start_data(&self, id: &str) -> Result<Message> {
		Ok(serialize(id)?.into())
	}

	pub fn update(&self, sk: &impl StereoKitDraw, buffer_manager: &mut BufferManager) {
		let frame_info = self.frame_info.lock();
		self.render_preview(sk, &*frame_info);
		self.render_dmabuf(sk, buffer_manager, &*frame_info);
	}

	fn render_preview(&self, sk: &impl StereoKitDraw, frame_info: &FrameInfo) {
		if !self.apply_preview_to.is_empty() {
			let sk_tex = self.sk_tex.get_or_init(|| {
				sk.tex_gen_color(
					Color128::default(),
					frame_info.preview_size.x as i32,
					frame_info.preview_size.y as i32,
					TextureType::RENDER_TARGET,
					TextureFormat::RGBA32Linear,
				)
			});
			let sk_mat = self.sk_mat.get_or_init(|| {
				let shader = sk.shader_create_mem(&UNLIT_SHADER_BYTES).unwrap();
				let mat = sk.material_create(&shader);
				sk.material_set_texture(&mat, "diffuse", sk_tex.as_ref());
				sk.material_set_transparency(&mat, Transparency::Blend);
				Arc::new(mat)
			});
			for model_part in self.apply_preview_to.take_valid_contents() {
				model_part.replace_material(sk_mat.clone())
			}
		}

		if !self.applied_preview_to.is_empty() {
			sk.render_to(
				self.sk_tex.get().unwrap(),
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

	fn render_dmabuf(
		&self,
		sk: &impl StereoKitDraw,
		buffer_manager: &mut BufferManager,
		frame_info: &FrameInfo,
	) {
		let mut render_notifiers = self.rendered_notifiers.lock();
		let mut render_requests_rx = self.render_requests_rx.lock();
		while let Ok((buffer_to_render, render_response_sender)) = render_requests_rx.try_recv() {
			let imported_dmabuf = buffer_manager
				.renderer
				.import_dmabuf(&buffer_to_render, None);
			let smithay_tex = match imported_dmabuf {
				Ok(t) => t,
				Err(e) => {
					let _ = render_response_sender.send(Err(ScenegraphError::MethodError {
						error: e.to_string(),
					}));
					continue;
				}
			};

			let sk_tex = sk.tex_create(TextureType::IMAGE_NO_MIPS, TextureFormat::RGBA32);
			unsafe {
				sk.tex_set_surface(
					&sk_tex,
					smithay_tex.tex_id() as usize as *mut c_void,
					TextureType::IMAGE_NO_MIPS,
					smithay::backend::renderer::gles::ffi::RGBA8.into(),
					buffer_to_render.size().w,
					buffer_to_render.size().h,
					1,
					false,
				);
			}

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
			render_notifiers.push(render_response_sender);
		}
	}

	pub fn send_rendered(&self) {
		for notifier in self.rendered_notifiers.lock().drain(..) {
			let _ = notifier.send(Ok(Vec::new().into()));
		}
	}
}

pub fn update(sk: &impl StereoKitDraw, buffer_manager: &mut BufferManager) {
	for camera in ITEM_TYPE_INFO_CAMERA.items.get_valid_contents() {
		let ItemType::Camera(camera) = &camera.specialization else {
			continue;
		};
		camera.update(sk, buffer_manager);
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
		preview_size: Vector2<u32>,
	}
	let info: CreateCameraItemInfo = deserialize(message.as_ref())?;
	let parent_name = format!("/item/{}/item", ITEM_TYPE_INFO_CAMERA.type_name);
	let space = find_spatial_parent(&calling_client, info.parent_path)?;
	let transform = parse_transform(info.transform, true, true, false);

	let node =
		Node::create(&INTERNAL_CLIENT, &parent_name, info.name, false).add_to_scenegraph()?;
	Spatial::add_to(&node, None, transform * space.global_transform(), false)?;
	CameraItem::add_to(&node, info.proj_matrix.into(), info.preview_size);
	node.item
		.get()
		.unwrap()
		.make_alias_named(&calling_client, &parent_name, info.name)?;
	Ok(())
}

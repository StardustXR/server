use super::core::Node;
use super::spatial::{get_spatial_parent_flex, Spatial};
use crate::core::client::Client;
use crate::core::registry::Registry;
use anyhow::{anyhow, ensure, Result};
use glam::Mat4;
use lazy_static::lazy_static;
use libstardustxr::{flex_to_quat, flex_to_vec3};
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use prisma::{Rgb, Rgba};
use send_wrapper::SendWrapper;
use std::fmt::Error;
use std::path::PathBuf;
use std::sync::Arc;
use stereokit::enums::RenderLayer;
use stereokit::lifecycle::DrawContext;
use stereokit::model::Model as SKModel;
use stereokit::StereoKit;

pub static MODEL_REGISTRY: Registry<Model> = Registry::new();
lazy_static! {
	pub static ref MODELS_TO_DROP: Mutex<Vec<SendWrapper<SKModel>>> = Default::default();
}

pub struct Model {
	space: Arc<Spatial>,
	pending_model_path: OnceCell<PathBuf>,
	sk_model: OnceCell<SendWrapper<SKModel>>,
}

impl Model {
	pub fn add_to(node: &Arc<Node>, path: String) -> Result<Arc<Model>> {
		ensure!(
			node.spatial.get().is_some(),
			"Internal: Node does not have a spatial attached!"
		);
		ensure!(
			node.model.get().is_none(),
			"Internal: Node already has a model attached!"
		);
		let model = Model {
			space: node.spatial.get().unwrap().clone(),
			pending_model_path: OnceCell::new(),
			sk_model: OnceCell::new(),
		};
		// node.add_local_method("", Spatial::get_transform_flex);
		let model_arc = MODEL_REGISTRY.add(model);
		let _ = model_arc.pending_model_path.set(PathBuf::from(path));
		let _ = node.model.set(model_arc.clone());
		Ok(model_arc)
	}

	pub fn draw(&self, sk: &StereoKit, draw_ctx: &DrawContext) {
		let sk_model = self
			.sk_model
			.get_or_try_init(|| {
				self.pending_model_path
					.get()
					.and_then(|path| SKModel::from_file(sk, path.as_path(), None))
					.map(|model| SendWrapper::new(model))
					.ok_or(Error)
			})
			.ok();

		if let Some(sk_model) = sk_model {
			sk_model.draw(
				draw_ctx,
				self.space.global_transform().into(),
				Rgba::new(Rgb::new(1_f32, 1_f32, 1_f32), 1_f32),
				RenderLayer::Layer0,
			);
		}
	}
}
impl Drop for Model {
	fn drop(&mut self) {
		if let Some(model) = self.sk_model.take() {
			MODELS_TO_DROP.lock().push(model);
		}
		MODEL_REGISTRY.remove(self);
	}
}

pub fn create_interface(client: &Arc<Client>) {
	let node = Node::create(client, "", "drawable", false);
	node.add_local_signal("createModelFromFile", create_from_file);
	node.add_to_scenegraph();
}

pub fn create_from_file(_node: &Node, calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
	let flex_vec = flexbuffers::Reader::get_root(data)?.get_vector()?;
	let node = Node::create(
		&calling_client,
		"/drawable/model",
		flex_vec.idx(0).get_str()?,
		true,
	);
	let parent = get_spatial_parent_flex(&calling_client, flex_vec.idx(1).get_str()?)?;
	let path = flex_vec.idx(2).get_str()?.to_string();
	let transform = Mat4::from_scale_rotation_translation(
		flex_to_vec3!(flex_vec.idx(5))
			.ok_or_else(|| anyhow!("Scale not found"))?
			.into(),
		flex_to_quat!(flex_vec.idx(4))
			.ok_or_else(|| anyhow!("Rotation not found"))?
			.into(),
		flex_to_vec3!(flex_vec.idx(3))
			.ok_or_else(|| anyhow!("Position not found"))?
			.into(),
	);
	let node = node.add_to_scenegraph();
	Spatial::add_to(&node, Some(parent), transform)?;
	Model::add_to(&node, path)?;
	Ok(())
}

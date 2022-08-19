use super::core::Node;
use super::spatial::{get_spatial_parent_flex, Spatial};
use crate::core::client::Client;
use crate::core::registry::Registry;
use anyhow::{anyhow, bail, ensure, Result};
use flexbuffers::FlexBufferType;
use glam::Mat4;
use lazy_static::lazy_static;
use libstardustxr::{flex_to_quat, flex_to_vec3};
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use prisma::{Rgb, Rgba};
use rustc_hash::FxHashMap;
use send_wrapper::SendWrapper;
use std::fmt::Error;
use std::path::PathBuf;
use std::sync::Arc;
use stereokit::enums::RenderLayer;
use stereokit::lifecycle::DrawContext;
use stereokit::model::Model as SKModel;
use stereokit::texture::Texture;
use stereokit::StereoKit;

pub static MODEL_REGISTRY: Registry<Model> = Registry::new();
lazy_static! {
	pub static ref MODELS_TO_DROP: Mutex<Vec<SendWrapper<SKModel>>> = Default::default();
}

pub enum MaterialParameter {
	Texture(PathBuf),
}

pub struct Model {
	space: Arc<Spatial>,
	pending_model_path: OnceCell<PathBuf>,
	pending_material_parameters: Mutex<FxHashMap<(u32, String), MaterialParameter>>,
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
			pending_material_parameters: Mutex::new(FxHashMap::default()),
			sk_model: OnceCell::new(),
		};
		node.add_local_signal("setMaterialParameter", Model::set_material_parameter);
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
			for ((material_idx, parameter_name), parameter_value) in
				self.pending_material_parameters.lock().iter()
			{
				if let Some(material) = sk_model.get_material(sk, *material_idx as i32) {
					match parameter_value {
						MaterialParameter::Texture(path) => {
							if let Some(tex) = Texture::from_file(sk, path.as_path(), true, 0) {
								material.set_parameter(parameter_name.as_str(), &tex);
							}
						}
					}
				}
			}
			self.pending_material_parameters.lock().clear();

			sk_model.draw(
				draw_ctx,
				self.space.global_transform().into(),
				Rgba::new(Rgb::new(1_f32, 1_f32, 1_f32), 1_f32),
				RenderLayer::Layer0,
			);
		}
	}

	fn set_material_parameter(
		node: &Node,
		_calling_client: Arc<Client>,
		data: &[u8],
	) -> Result<()> {
		let model = node.model.get().unwrap();
		let flex_vec = flexbuffers::Reader::get_root(data)?.get_vector()?;
		let material_idx = flex_vec
			.idx(0)
			.get_u64()
			.map_err(|_| anyhow!("Material ID is not a number!"))? as u32;
		let parameter_name = flex_vec
			.idx(1)
			.get_str()
			.map_err(|_| anyhow!("Parameter name is not a string!"))?;

		let flex_parameter_value = flex_vec.idx(2);
		let parameter_value = match flex_parameter_value.flexbuffer_type() {
			FlexBufferType::String => {
				MaterialParameter::Texture(PathBuf::from(flex_parameter_value.as_str()))
			}
			_ => bail!("Invalid parameter value type"),
		};

		model
			.pending_material_parameters
			.lock()
			.insert((material_idx, parameter_name.to_string()), parameter_value);

		Ok(())
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

use super::Node;
use crate::core::client::Client;
use crate::core::destroy_queue;
use crate::core::registry::Registry;
use crate::core::resource::{parse_resource_id, ResourceID};
use crate::nodes::spatial::{get_spatial_parent_flex, parse_transform, Spatial};
use anyhow::{anyhow, bail, ensure, Result};
use flexbuffers::FlexBufferType;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use prisma::{Rgb, Rgba};
use rustc_hash::FxHashMap;
use send_wrapper::SendWrapper;
use std::fmt::Error;
use std::path::PathBuf;
use std::sync::Arc;
use stereokit::lifecycle::DrawContext;
use stereokit::material::Material;
use stereokit::model::Model as SKModel;
use stereokit::render::RenderLayer;
use stereokit::texture::Texture;
use stereokit::StereoKit;

static MODEL_REGISTRY: Registry<Model> = Registry::new();

pub enum MaterialParameter {
	Texture(PathBuf),
}

pub struct Model {
	space: Arc<Spatial>,
	resource_id: ResourceID,
	pending_model_path: OnceCell<PathBuf>,
	pending_material_parameters: Mutex<FxHashMap<(u32, String), MaterialParameter>>,
	pub pending_material_replacements: Mutex<FxHashMap<u32, Arc<SendWrapper<Material>>>>,
	sk_model: OnceCell<SendWrapper<SKModel>>,
}

impl Model {
	pub fn add_to(node: &Arc<Node>, resource_id: ResourceID) -> Result<Arc<Model>> {
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
			resource_id,
			pending_model_path: OnceCell::new(),
			pending_material_parameters: Mutex::new(FxHashMap::default()),
			pending_material_replacements: Mutex::new(FxHashMap::default()),
			sk_model: OnceCell::new(),
		};
		node.add_local_signal("setMaterialParameter", Model::set_material_parameter);
		let model_arc = MODEL_REGISTRY.add(model);
		let _ = model_arc.pending_model_path.set(
			model_arc
				.resource_id
				.get_file(&node.get_client().base_resource_prefixes.lock().clone())
				.ok_or_else(|| anyhow!("Resource not found"))?,
		);
		let _ = node.model.set(model_arc.clone());
		Ok(model_arc)
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

	fn draw(&self, sk: &StereoKit, draw_ctx: &DrawContext) {
		let sk_model = self
			.sk_model
			.get_or_try_init(|| {
				self.pending_model_path
					.get()
					.and_then(|path| SKModel::from_file(sk, path.as_path(), None))
					.as_ref()
					.cloned()
					.map(SendWrapper::new)
					.ok_or(Error)
			})
			.ok();

		if let Some(sk_model) = sk_model {
			{
				let mut material_replacements = self.pending_material_replacements.lock();
				for (material_idx, replacement_material) in material_replacements.iter() {
					sk_model.set_material(*material_idx as i32, replacement_material);
				}
				material_replacements.clear();
			}

			{
				let mut material_parameters = self.pending_material_parameters.lock();
				for ((material_idx, parameter_name), parameter_value) in material_parameters.iter()
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
				material_parameters.clear();
			}

			let global_transform = self.space.global_transform().into();
			sk_model.draw(
				draw_ctx,
				global_transform,
				Rgba::new(Rgb::new(1_f32, 1_f32, 1_f32), 1_f32),
				RenderLayer::Layer0,
			);
		}
	}
}
impl Drop for Model {
	fn drop(&mut self) {
		if let Some(model) = self.sk_model.take() {
			destroy_queue::add(model);
		}
		MODEL_REGISTRY.remove(self);
	}
}

pub fn draw_all(sk: &StereoKit, draw_ctx: &DrawContext) {
	for model in MODEL_REGISTRY.get_valid_contents() {
		model.draw(sk, draw_ctx);
	}
}

pub fn create_flex(_node: &Node, calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
	let flex_vec = flexbuffers::Reader::get_root(data)?.get_vector()?;
	let node = Node::create(
		&calling_client,
		"/drawable/model",
		flex_vec.idx(0).get_str()?,
		true,
	);
	let parent = get_spatial_parent_flex(&calling_client, flex_vec.idx(1).get_str()?)?;
	let transform = parse_transform(flex_vec.index(2)?, true, true, true)?;
	let resource_id = parse_resource_id(flex_vec.idx(3))?;
	let node = node.add_to_scenegraph();
	Spatial::add_to(&node, Some(parent), transform)?;
	Model::add_to(&node, resource_id)?;
	Ok(())
}

use super::Node;
use crate::core::client::Client;
use crate::core::destroy_queue;
use crate::core::registry::Registry;
use crate::core::resource::ResourceID;
use crate::nodes::drawable::Drawable;
use crate::nodes::spatial::{find_spatial_parent, parse_transform, Spatial};
use color_eyre::eyre::{bail, ensure, eyre, Result};
use mint::{ColumnMatrix4, Vector2, Vector3, Vector4};
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use portable_atomic::{AtomicBool, Ordering};
use rustc_hash::FxHashMap;
use send_wrapper::SendWrapper;
use serde::Deserialize;
use stardust_xr::schemas::flex::deserialize;
use stardust_xr::values::Transform;
use std::ffi::OsStr;
use std::fmt::Error;
use std::path::PathBuf;
use std::sync::Arc;
use stereokit::color_named::WHITE;
use stereokit::lifecycle::{StereoKitContext, StereoKitDraw};
use stereokit::material::Material;
use stereokit::model::Model as SKModel;
use stereokit::render::RenderLayer;
use stereokit::texture::Texture;
use stereokit::values::Color128;

static MODEL_REGISTRY: Registry<Model> = Registry::new();

#[derive(Deserialize, Debug)]
#[serde(tag = "t", content = "c")]
pub enum MaterialParameter {
	Float(f32),
	Vector2(Vector2<f32>),
	Vector3(Vector3<f32>),
	Vector4(Vector4<f32>),
	Color([f32; 4]),
	Int(i32),
	Int2(Vector2<i32>),
	Int3(Vector3<i32>),
	Int4(Vector4<i32>),
	Bool(bool),
	UInt(u32),
	UInt2(Vector2<u32>),
	UInt3(Vector3<u32>),
	UInt4(Vector4<u32>),
	Matrix(ColumnMatrix4<f32>),
	Texture(ResourceID),
}
impl MaterialParameter {
	fn apply_to_material(
		&self,
		client: &Client,
		sk: &impl StereoKitContext,
		material: &Material,
		parameter_name: &str,
	) {
		match self {
			MaterialParameter::Float(val) => {
				material.set_parameter(sk, parameter_name, val);
			}
			MaterialParameter::Vector2(val) => {
				material.set_parameter(sk, parameter_name, val);
			}
			MaterialParameter::Vector3(val) => {
				material.set_parameter(sk, parameter_name, val);
			}
			MaterialParameter::Vector4(val) => {
				material.set_parameter(sk, parameter_name, val);
			}
			MaterialParameter::Color(val) => {
				material.set_parameter(sk, parameter_name, &Color128::from(val.clone()));
			}
			MaterialParameter::Int(val) => {
				material.set_parameter(sk, parameter_name, val);
			}
			MaterialParameter::Int2(val) => {
				material.set_parameter(sk, parameter_name, val);
			}
			MaterialParameter::Int3(val) => {
				material.set_parameter(sk, parameter_name, val);
			}
			MaterialParameter::Int4(val) => {
				material.set_parameter(sk, parameter_name, val);
			}
			MaterialParameter::Bool(val) => {
				material.set_parameter(sk, parameter_name, val);
			}
			MaterialParameter::UInt(val) => {
				material.set_parameter(sk, parameter_name, val);
			}
			MaterialParameter::UInt2(val) => {
				material.set_parameter(sk, parameter_name, val);
			}
			MaterialParameter::UInt3(val) => {
				material.set_parameter(sk, parameter_name, val);
			}
			MaterialParameter::UInt4(val) => {
				material.set_parameter(sk, parameter_name, val);
			}
			MaterialParameter::Matrix(val) => {
				material.set_parameter(sk, parameter_name, val);
			}
			MaterialParameter::Texture(resource) => {
				let Some(texture_path) = resource.get_file(
					&client.base_resource_prefixes.lock().clone(),
					&[OsStr::new("png"), OsStr::new("jpg")],
				) else { return; };
				if let Some(tex) = Texture::from_file(sk, texture_path, true, 0) {
					material.set_parameter(sk, parameter_name, &tex);
				}
			}
		}
	}
}

pub struct Model {
	enabled: Arc<AtomicBool>,
	space: Arc<Spatial>,
	resource_id: ResourceID,
	pending_model_path: OnceCell<PathBuf>,
	pending_material_parameters: Mutex<FxHashMap<(i32, String), MaterialParameter>>,
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
			node.drawable.get().is_none(),
			"Internal: Node already has a drawable attached!"
		);
		let model = Model {
			enabled: node.enabled.clone(),
			space: node.spatial.get().unwrap().clone(),
			resource_id,
			pending_model_path: OnceCell::new(),
			pending_material_parameters: Mutex::new(FxHashMap::default()),
			pending_material_replacements: Mutex::new(FxHashMap::default()),
			sk_model: OnceCell::new(),
		};
		node.add_local_signal("set_material_parameter", Model::set_material_parameter_flex);
		let model_arc = MODEL_REGISTRY.add(model);
		let _ = model_arc.pending_model_path.set(
			model_arc
				.resource_id
				.get_file(
					&node
						.get_client()
						.ok_or_else(|| eyre!("Client not found"))?
						.base_resource_prefixes
						.lock()
						.clone(),
					&[OsStr::new("glb"), OsStr::new("gltf")],
				)
				.ok_or_else(|| eyre!("Resource not found"))?,
		);
		let _ = node.drawable.set(Drawable::Model(model_arc.clone()));
		Ok(model_arc)
	}

	fn set_material_parameter_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		data: &[u8],
	) -> Result<()> {
		let Some(Drawable::Model(model)) = node.drawable.get() else {bail!("Not a drawable??")};

		#[derive(Deserialize)]
		struct MaterialParameterInfo {
			idx: u32,
			name: String,
			value: MaterialParameter,
		}
		let info: MaterialParameterInfo = deserialize(data)?;

		model
			.pending_material_parameters
			.lock()
			.insert((info.idx as i32, info.name), info.value);

		Ok(())
	}

	fn draw(&self, sk: &StereoKitDraw) {
		let sk_model = self
			.sk_model
			.get_or_try_init(|| -> color_eyre::eyre::Result<SendWrapper<SKModel>> {
				let pending_model_path = self.pending_model_path.get().ok_or(Error)?;
				let model = SKModel::from_file(sk, pending_model_path.as_path(), None)?;

				Ok(SendWrapper::new(model.clone()))
			})
			.ok();

		if let Some(sk_model) = sk_model {
			{
				let mut material_replacements = self.pending_material_replacements.lock();
				for (material_idx, replacement_material) in material_replacements.iter() {
					if sk_model.get_material(sk, *material_idx as i32).is_some() {
						sk_model.set_material(sk, *material_idx as i32, replacement_material);
					}
				}
				material_replacements.clear();
			}

			if let Some(client) = self.space.node.upgrade().and_then(|n| n.client.upgrade()) {
				let mut material_parameters = self.pending_material_parameters.lock();
				for ((material_idx, parameter_name), parameter_value) in material_parameters.drain()
				{
					let Some(material) = sk_model.get_material(sk, material_idx) else {continue};
					let new_material = material.clone();
					parameter_value.apply_to_material(
						&client,
						sk,
						&new_material,
						parameter_name.as_str(),
					);
					sk_model.set_material(sk, material_idx, &new_material);
				}
			}

			let global_transform = self.space.global_transform().into();
			sk_model.draw(sk, global_transform, WHITE, RenderLayer::Layer0);
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

pub fn draw_all(sk: &StereoKitDraw) {
	for model in MODEL_REGISTRY.get_valid_contents() {
		if model.enabled.load(Ordering::Relaxed) {
			model.draw(sk);
		}
	}
}

pub fn create_flex(_node: &Node, calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
	#[derive(Deserialize)]
	struct CreateModelInfo<'a> {
		name: &'a str,
		parent_path: &'a str,
		transform: Transform,
		resource: ResourceID,
	}
	let info: CreateModelInfo = deserialize(data)?;
	let node = Node::create(&calling_client, "/drawable/model", info.name, true);
	let parent = find_spatial_parent(&calling_client, info.parent_path)?;
	let transform = parse_transform(info.transform, true, true, true);
	let node = node.add_to_scenegraph()?;
	Spatial::add_to(&node, Some(parent), transform, false)?;
	Model::add_to(&node, info.resource)?;
	Ok(())
}

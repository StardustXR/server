use super::Node;
use crate::core::client::Client;
use crate::core::node_collections::LifeLinkedNodeMap;
use crate::core::registry::Registry;
use crate::core::resource::ResourceID;
use crate::nodes::drawable::Drawable;
use crate::nodes::spatial::{find_spatial_parent, parse_transform, Spatial};
use crate::SK_MULTITHREAD;
use color_eyre::eyre::{bail, ensure, eyre, Result};
use glam::Mat4;
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
use std::path::PathBuf;
use std::sync::{Arc, Weak};
use stereokit::named_colors::WHITE;
use stereokit::{
	Color128, Material, Model as SKModel, RenderLayer, Shader, StereoKitDraw, StereoKitMultiThread,
};

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
		sk: &impl StereoKitMultiThread,
		material: &Material,
		parameter_name: &str,
	) {
		match self {
			MaterialParameter::Float(val) => {
				sk.material_set_float(material, parameter_name, *val);
			}
			MaterialParameter::Vector2(val) => {
				sk.material_set_vector2(material, parameter_name, *val);
			}
			MaterialParameter::Vector3(val) => {
				sk.material_set_vector3(material, parameter_name, *val);
			}
			MaterialParameter::Vector4(val) => {
				sk.material_set_vector4(material, parameter_name, *val);
			}
			MaterialParameter::Color(val) => {
				sk.material_set_color(material, parameter_name, Color128::from(val.clone()));
			}
			MaterialParameter::Int(val) => {
				sk.material_set_int(material, parameter_name, *val);
			}
			MaterialParameter::Int2(val) => {
				sk.material_set_int2(material, parameter_name, val.x, val.y);
			}
			MaterialParameter::Int3(val) => {
				sk.material_set_int3(material, parameter_name, val.x, val.y, val.z);
			}
			MaterialParameter::Int4(val) => {
				sk.material_set_int4(material, parameter_name, val.w, val.x, val.y, val.z);
			}
			MaterialParameter::Bool(val) => {
				sk.material_set_bool(material, parameter_name, *val);
			}
			MaterialParameter::UInt(val) => {
				sk.material_set_uint(material, parameter_name, *val);
			}
			MaterialParameter::UInt2(val) => {
				sk.material_set_uint2(material, parameter_name, val.x, val.y);
			}
			MaterialParameter::UInt3(val) => {
				sk.material_set_uint3(material, parameter_name, val.x, val.y, val.z);
			}
			MaterialParameter::UInt4(val) => {
				sk.material_set_uint4(material, parameter_name, val.w, val.x, val.y, val.z);
			}
			MaterialParameter::Matrix(val) => {
				sk.material_set_matrix(material, parameter_name, Mat4::from(*val));
			}
			MaterialParameter::Texture(resource) => {
				let Some(texture_path) = resource.get_file(
					&client.base_resource_prefixes.lock().clone(),
					&[OsStr::new("png"), OsStr::new("jpg")],
				) else {return};
				if let Ok(tex) = sk.tex_create_file(texture_path, true, 0) {
					sk.material_set_texture(material, parameter_name, &tex);
				}
			}
		}
	}
}

pub struct ModelPart {
	id: i32,
	path: PathBuf,
	space: Arc<Spatial>,
	model: Weak<Model>,
	pending_material_parameters: Mutex<FxHashMap<String, MaterialParameter>>,
	pending_material_replacement: Mutex<Option<Arc<SendWrapper<Material>>>>,
}
impl ModelPart {
	fn create_for_model(sk: &impl StereoKitMultiThread, model: &Arc<Model>, sk_model: &SKModel) {
		let first_root_part = sk.model_node_get_root(sk_model);
		let mut current_option_part = Some(first_root_part);

		while let Some(current_part) = &mut current_option_part {
			ModelPart::create(sk, model, sk_model, *current_part);

			if let Some(child) = sk.model_node_child(sk_model, *current_part) {
				*current_part = child;
			} else if let Some(sibling) = sk.model_node_sibling(sk_model, *current_part) {
				*current_part = sibling;
			} else {
				while let Some(current_part) = &mut current_option_part {
					if let Some(sibling) = sk.model_node_sibling(sk_model, *current_part) {
						*current_part = sibling;
						break;
					}
					current_option_part = sk.model_node_parent(sk_model, *current_part);
				}
			}
		}
	}

	fn create(
		sk: &impl StereoKitMultiThread,
		model: &Arc<Model>,
		sk_model: &SKModel,
		id: i32,
	) -> Option<Arc<Self>> {
		let parent_node = sk
			.model_node_parent(sk_model, id)
			.and_then(|id| model.parts.get(&id));
		let parent_part = parent_node
			.as_ref()
			.and_then(|node| match node.drawable.get() {
				Some(Drawable::ModelPart(model_part)) => Some(model_part),
				_ => None,
			});

		let stardust_model_part = model.space.node()?;
		let client = stardust_model_part.get_client()?;
		let mut part_path = parent_part.map(|n| n.path.clone()).unwrap_or_default();
		part_path.push(sk.model_node_get_name(sk_model, id)?);
		let node = client.scenegraph.add_node(Node::create(
			&client,
			stardust_model_part.get_path(),
			part_path.to_str()?,
			false,
		));
		let spatial_parent = parent_node
			.and_then(|n| n.spatial.get().cloned())
			.unwrap_or_else(|| model.space.clone());
		let space = Spatial::add_to(
			&node,
			Some(spatial_parent),
			sk.model_node_get_transform_local(sk_model, id),
			false,
		)
		.ok()?;
		let model_part = Arc::new(ModelPart {
			id,
			path: part_path,
			space,
			model: Arc::downgrade(model),
			pending_material_parameters: Mutex::new(FxHashMap::default()),
			pending_material_replacement: Mutex::new(None),
		});
		node.add_local_signal(
			"set_material_parameter",
			ModelPart::set_material_parameter_flex,
		);
		let _ = node.drawable.set(Drawable::ModelPart(model_part.clone()));
		model.parts.add(id, &node);
		Some(model_part)
	}

	fn set_material_parameter_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		data: &[u8],
	) -> Result<()> {
		let Some(Drawable::ModelPart(model_part)) = node.drawable.get() else {bail!("Not a drawable??")};

		let (name, value): (String, MaterialParameter) = deserialize(data)?;

		model_part
			.pending_material_parameters
			.lock()
			.insert(name, value);

		Ok(())
	}

	pub fn replace_material(&self, replacement: Arc<SendWrapper<Material>>) {
		self.pending_material_replacement
			.lock()
			.replace(replacement);
	}

	fn update(&self, sk: &impl StereoKitDraw) {
		let Some(model) = self.model.upgrade() else {return};
		let Some(sk_model) = model.sk_model.get() else {return};
		let Some(node) = model.space.node() else {return};
		let Some(client) = node.get_client() else {return};
		if let Some(material_replacement) = self.pending_material_replacement.lock().take() {
			sk.model_node_set_material(sk_model, self.id, material_replacement.as_ref().as_ref());
		}

		let mut material_parameters = self.pending_material_parameters.lock();
		for (parameter_name, parameter_value) in material_parameters.drain() {
			let Some(material) = sk.model_node_get_material(sk_model, self.id) else {continue};
			let new_material = sk.material_copy(material);
			parameter_value.apply_to_material(&client, sk, &new_material, parameter_name.as_str());
			sk.model_node_set_material(sk_model, self.id, &new_material);
		}

		sk.model_node_set_transform_model(
			sk_model,
			self.id,
			Spatial::space_to_space_matrix(Some(&self.space), Some(&model.space)),
		);
	}
}

pub struct Model {
	self_ref: Weak<Model>,
	enabled: Arc<AtomicBool>,
	space: Arc<Spatial>,
	_resource_id: ResourceID,
	sk_model: OnceCell<SKModel>,
	parts: LifeLinkedNodeMap<i32>,
}
unsafe impl Send for Model {}
unsafe impl Sync for Model {}

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

		let pending_model_path = resource_id
			.get_file(
				&node
					.get_client()
					.ok_or_else(|| eyre!("Client not found"))?
					.base_resource_prefixes
					.lock()
					.clone(),
				&[OsStr::new("glb"), OsStr::new("gltf")],
			)
			.ok_or_else(|| eyre!("Resource not found"))?;

		let model = Arc::new_cyclic(|self_ref| Model {
			self_ref: self_ref.clone(),
			enabled: node.enabled.clone(),
			space: node.spatial.get().unwrap().clone(),
			_resource_id: resource_id,
			sk_model: OnceCell::new(),
			parts: LifeLinkedNodeMap::default(),
		});
		MODEL_REGISTRY.add_raw(&model);

		let sk = SK_MULTITHREAD.get().unwrap();
		let sk_model =
			sk.model_create_file(pending_model_path.to_str().unwrap(), None::<Shader>)?;
		ModelPart::create_for_model(sk, &model.self_ref.upgrade().unwrap(), &sk_model);
		let _ = model.sk_model.set(sk_model);
		let _ = node.drawable.set(Drawable::Model(model.clone()));
		Ok(model)
	}

	fn draw(&self, sk: &impl StereoKitDraw) {
		let Some(sk_model) = self.sk_model.get() else {return};
		for model_node_node in self.parts.nodes() {
			let Some(Drawable::ModelPart(model_node)) = model_node_node.drawable.get() else {continue};
			model_node.update(sk);
		}

		sk.model_draw(
			sk_model,
			self.space.global_transform(),
			WHITE,
			RenderLayer::LAYER0,
		);
	}
}

impl Drop for Model {
	fn drop(&mut self) {
		MODEL_REGISTRY.remove(self);
	}
}

pub fn draw_all(sk: &impl StereoKitDraw) {
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

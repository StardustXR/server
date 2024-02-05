use super::{MaterialParameter, ModelAspect, ModelPartAspect, Node};
use crate::core::client::Client;
use crate::core::destroy_queue;
use crate::core::node_collections::LifeLinkedNodeMap;
use crate::core::registry::Registry;
use crate::core::resource::get_resource_file;
use crate::nodes::spatial::Spatial;
use crate::nodes::Aspect;
use crate::SK_MULTITHREAD;
use color_eyre::eyre::{eyre, Result};
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use portable_atomic::{AtomicBool, Ordering};
use rustc_hash::FxHashMap;
use stardust_xr::values::ResourceID;

use std::ffi::OsStr;
use std::path::PathBuf;
use std::sync::{Arc, Weak};
use stereokit::named_colors::WHITE;
use stereokit::{
	Bounds, Color128, Material, Model as SKModel, RenderLayer, Shader, StereoKitDraw,
	StereoKitMultiThread, Transparency,
};

static MODEL_REGISTRY: Registry<Model> = Registry::new();
static HOLDOUT_MATERIAL: OnceCell<Arc<Material>> = OnceCell::new();

impl MaterialParameter {
	fn apply_to_material(
		&self,
		client: &Client,
		sk: &impl StereoKitMultiThread,
		material: &Material,
		parameter_name: &str,
	) {
		match self {
			MaterialParameter::Bool(val) => {
				sk.material_set_bool(material, parameter_name, *val);
			}
			MaterialParameter::Int(val) => {
				sk.material_set_int(material, parameter_name, *val);
			}
			MaterialParameter::UInt(val) => {
				sk.material_set_uint(material, parameter_name, *val);
			}
			MaterialParameter::Float(val) => {
				sk.material_set_float(material, parameter_name, *val);
			}
			MaterialParameter::Vec2(val) => {
				sk.material_set_vector2(material, parameter_name, *val);
			}
			MaterialParameter::Vec3(val) => {
				sk.material_set_vector3(material, parameter_name, *val);
			}
			MaterialParameter::Color(val) => {
				sk.material_set_color(
					material,
					parameter_name,
					Color128::new(val.c.r, val.c.g, val.c.b, val.a),
				);
			}
			MaterialParameter::Texture(resource) => {
				let Some(texture_path) =
					get_resource_file(&resource, &client, &[OsStr::new("png"), OsStr::new("jpg")])
				else {
					return;
				};
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
	pending_material_replacement: Mutex<Option<Arc<Material>>>,
}
impl ModelPart {
	fn create_for_model(sk: &impl StereoKitMultiThread, model: &Arc<Model>, sk_model: &SKModel) {
		HOLDOUT_MATERIAL.get_or_init(|| {
			let mat = sk.material_copy(Material::UNLIT);
			sk.material_set_transparency(&mat, Transparency::None);
			sk.material_set_color(
				&mat,
				"color",
				stereokit::sys::color128 {
					r: 0.0,
					g: 0.0,
					b: 0.0,
					a: 0.0,
				},
			);
			Arc::new(mat)
		});

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
			.and_then(|node| node.get_aspect::<ModelPart>().ok());

		let stardust_model_part = model.space.node()?;
		let client = stardust_model_part.get_client()?;
		let mut part_path = parent_part.map(|n| n.path.clone()).unwrap_or_default();
		part_path.push(sk.model_node_get_name(sk_model, id)?);
		let node = client.scenegraph.add_node(Node::create_parent_name(
			&client,
			stardust_model_part.get_path(),
			part_path.to_str()?,
			false,
		));
		let spatial_parent = parent_node
			.and_then(|n| n.get_aspect::<Spatial>().ok())
			.unwrap_or_else(|| model.space.clone());
		let space = Spatial::add_to(
			&node,
			Some(spatial_parent),
			sk.model_node_get_transform_local(sk_model, id),
			false,
		);

		let _ = node
			.get_aspect::<Spatial>()
			.unwrap()
			.bounding_box_calc
			.set(|node| {
				let Ok(model_part) = node.get_aspect::<ModelPart>() else {
					return Bounds::default();
				};
				let Some(sk) = SK_MULTITHREAD.get() else {
					return Bounds::default();
				};
				let Some(model) = model_part.model.upgrade() else {
					return Bounds::default();
				};
				let Some(sk_model) = model.sk_model.get() else {
					return Bounds::default();
				};
				let Some(sk_mesh) = sk.model_node_get_mesh(sk_model, model_part.id) else {
					return Bounds::default();
				};
				sk.mesh_get_bounds(sk_mesh)
			});

		let model_part = Arc::new(ModelPart {
			id,
			path: part_path,
			space,
			model: Arc::downgrade(model),
			pending_material_parameters: Mutex::new(FxHashMap::default()),
			pending_material_replacement: Mutex::new(None),
		});
		<ModelPart as ModelPartAspect>::add_node_members(&node);
		node.add_aspect_raw(model_part.clone());
		model.parts.add(id, &node);
		Some(model_part)
	}

	pub fn replace_material(&self, replacement: Arc<Material>) {
		self.pending_material_replacement
			.lock()
			.replace(replacement);
	}

	fn update(&self, sk: &impl StereoKitDraw) {
		let Some(model) = self.model.upgrade() else {
			return;
		};
		let Some(sk_model) = model.sk_model.get() else {
			return;
		};
		let Some(node) = model.space.node() else {
			return;
		};
		let Some(client) = node.get_client() else {
			return;
		};
		if let Some(material_replacement) = self.pending_material_replacement.lock().take() {
			sk.model_node_set_material(sk_model, self.id, material_replacement.as_ref().as_ref());
		}

		let mut material_parameters = self.pending_material_parameters.lock();
		for (parameter_name, parameter_value) in material_parameters.drain() {
			let Some(material) = sk.model_node_get_material(sk_model, self.id) else {
				continue;
			};
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
impl Aspect for ModelPart {
	const NAME: &'static str = "ModelPart";
}
impl ModelPartAspect for ModelPart {
	#[doc = "Set this model part's material to one that cuts a hole in the world. Often used for overlays/passthrough where you want to show the background through an object."]
	fn apply_holdout_material(node: Arc<Node>, _calling_client: Arc<Client>) -> Result<()> {
		let model_part = node.get_aspect::<ModelPart>()?;
		model_part.replace_material(HOLDOUT_MATERIAL.get().unwrap().clone());
		Ok(())
	}

	#[doc = "Set the material parameter with `parameter_name` to `value`"]
	fn set_material_parameter(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		parameter_name: String,
		value: MaterialParameter,
	) -> Result<()> {
		let model_part = node.get_aspect::<ModelPart>()?;
		model_part
			.pending_material_parameters
			.lock()
			.insert(parameter_name, value);

		Ok(())
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
		let pending_model_path = get_resource_file(
			&resource_id,
			&*node.get_client().ok_or_else(|| eyre!("Client not found"))?,
			&[OsStr::new("glb"), OsStr::new("gltf")],
		)
		.ok_or_else(|| eyre!("Resource not found"))?;

		let model = Arc::new_cyclic(|self_ref| Model {
			self_ref: self_ref.clone(),
			enabled: node.enabled.clone(),
			space: node.get_aspect::<Spatial>().unwrap().clone(),
			_resource_id: resource_id,
			sk_model: OnceCell::new(),
			parts: LifeLinkedNodeMap::default(),
		});
		MODEL_REGISTRY.add_raw(&model);

		let sk = SK_MULTITHREAD.get().unwrap();
		let sk_model = sk.model_copy(
			sk.model_create_file(pending_model_path.to_str().unwrap(), None::<Shader>)?,
		);
		ModelPart::create_for_model(sk, &model.self_ref.upgrade().unwrap(), &sk_model);
		let _ = model.sk_model.set(sk_model);
		node.add_aspect_raw(model.clone());
		Ok(model)
	}

	fn draw(&self, sk: &impl StereoKitDraw) {
		let Some(sk_model) = self.sk_model.get() else {
			return;
		};
		for model_node_node in self.parts.nodes() {
			if let Ok(model_node) = model_node_node.get_aspect::<ModelPart>() {
				model_node.update(sk);
			};
		}

		sk.model_draw(
			sk_model,
			self.space.global_transform(),
			WHITE,
			RenderLayer::LAYER0,
		);
	}
}
impl Aspect for Model {
	const NAME: &'static str = "Model";
}
impl ModelAspect for Model {}
impl Drop for Model {
	fn drop(&mut self) {
		if let Some(sk_model) = self.sk_model.take() {
			destroy_queue::add(sk_model);
		}
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

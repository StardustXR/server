use super::{MaterialParameter, ModelAspect, ModelPartAspect, Node};
use crate::core::client::Client;
use crate::core::node_collections::LifeLinkedNodeMap;
use crate::core::registry::Registry;
use crate::core::resource::get_resource_file;
use crate::nodes::spatial::Spatial;
use crate::nodes::Aspect;
use color_eyre::eyre::{eyre, Result};
use glam::{Mat4, Vec2, Vec3};
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use portable_atomic::{AtomicBool, Ordering};
use rustc_hash::FxHashMap;
use send_wrapper::SendWrapper;
use stardust_xr::values::ResourceID;
use stereokit_rust::material::Transparency;
use stereokit_rust::maths::Bounds;
use stereokit_rust::sk::MainThreadToken;
use stereokit_rust::{material::Material, model::Model as SKModel, tex::Tex, util::Color128};

use std::ffi::OsStr;
use std::path::PathBuf;
use std::sync::{Arc, Weak};

static MODEL_REGISTRY: Registry<Model> = Registry::new();
static HOLDOUT_MATERIAL: OnceCell<Arc<SendWrapper<Material>>> = OnceCell::new();

impl MaterialParameter {
	fn apply_to_material(&self, client: &Client, material: &Material, parameter_name: &str) {
		let mut params = material.get_all_param_info();
		match self {
			MaterialParameter::Bool(val) => {
				params.set_bool(parameter_name, *val);
			}
			MaterialParameter::Int(val) => {
				params.set_int(parameter_name, &[*val]);
			}
			MaterialParameter::UInt(val) => {
				params.set_uint(parameter_name, &[*val]);
			}
			MaterialParameter::Float(val) => {
				params.set_float(parameter_name, *val);
			}
			MaterialParameter::Vec2(val) => {
				params.set_vec2(parameter_name, Vec2::from(*val));
			}
			MaterialParameter::Vec3(val) => {
				params.set_vec3(parameter_name, Vec3::from(*val));
			}
			MaterialParameter::Color(val) => {
				params.set_color(
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
				if let Ok(tex) = Tex::from_file(texture_path, true, None) {
					params.set_texture(parameter_name, &tex);
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
	fn create_for_model(model: &Arc<Model>, sk_model: &SKModel) {
		HOLDOUT_MATERIAL.get_or_init(|| {
			let mut mat = Material::copy(Material::unlit());
			mat.transparency(Transparency::None);
			mat.color_tint(Color128::BLACK_TRANSPARENT);
			Arc::new(SendWrapper::new(mat))
		});

		let nodes = sk_model.get_nodes();
		for part in nodes.all() {
			ModelPart::create(model, &part);
		}
	}

	fn create(model: &Arc<Model>, part: &stereokit_rust::model::ModelNode) -> Option<Arc<Self>> {
		let parent_node = part
			.get_parent()
			.and_then(|part| model.parts.get(part.get_id()));
		let parent_part = parent_node
			.as_ref()
			.and_then(|node| node.get_aspect::<ModelPart>().ok());

		let stardust_model_part = model.space.node()?;
		let client = stardust_model_part.get_client()?;
		let mut part_path = parent_part.map(|n| n.path.clone()).unwrap_or_default();
		part_path.push(part.get_name().unwrap());

		let node = client.scenegraph.add_node(Node::create_parent_name(
			&client,
			stardust_model_part.get_path(),
			part_path.to_str()?,
			false,
		));
		let spatial_parent = parent_node
			.and_then(|n| n.get_aspect::<Spatial>().ok())
			.unwrap_or_else(|| model.space.clone());

		let local_transform = unsafe { part.get_local_transform().m };
		let space = Spatial::add_to(
			&node,
			Some(spatial_parent),
			Mat4::from_cols_array(&local_transform),
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
				let Some(model) = model_part.model.upgrade() else {
					return Bounds::default();
				};
				let Some(sk_model) = model.sk_model.get() else {
					return Bounds::default();
				};
				let nodes = sk_model.get_nodes();
				let Some(model_node) = nodes.get_index(model_part.id) else {
					return Bounds::default();
				};
				let Some(sk_mesh) = model_node.get_mesh() else {
					return Bounds::default();
				};
				sk_mesh.get_bounds()
			});

		let model_part = Arc::new(ModelPart {
			id: *part.get_id(),
			path: part_path,
			space,
			model: Arc::downgrade(model),
			pending_material_parameters: Mutex::new(FxHashMap::default()),
			pending_material_replacement: Mutex::new(None),
		});
		<ModelPart as ModelPartAspect>::add_node_members(&node);
		node.add_aspect_raw(model_part.clone());
		model.parts.add(*part.get_id(), &node);
		Some(model_part)
	}

	pub fn replace_material(&self, replacement: Arc<SendWrapper<Material>>) {
		self.pending_material_replacement
			.lock()
			.replace(replacement);
	}
	/// only to be run on the main thread
	pub fn replace_material_now(&self, replacement: &Material) {
		let Some(model) = self.model.upgrade() else {
			return;
		};
		let Some(sk_model) = model.sk_model.get() else {
			return;
		};
		let nodes = sk_model.get_nodes();
		let Some(mut part) = nodes.get_index(self.id) else {
			return;
		};
		part.material(replacement);
	}

	fn update(&self) {
		let Some(model) = self.model.upgrade() else {
			return;
		};
		let Some(sk_model) = model.sk_model.get() else {
			return;
		};
		let Some(node) = model.space.node() else {
			return;
		};
		let nodes = sk_model.get_nodes();
		let Some(mut part) = nodes.get_index(self.id) else {
			return;
		};
		part.model_transform(Spatial::space_to_space_matrix(
			Some(&self.space),
			Some(&model.space),
		));

		let Some(client) = node.get_client() else {
			return;
		};

		if let Some(material_replacement) = self.pending_material_replacement.lock().take() {
			part.material(&**material_replacement);
		}

		// todo: find all materials with identical parameters and batch them into 1 material again
		'mat_params: {
			let mut material_parameters = self.pending_material_parameters.lock();
			if !material_parameters.is_empty() {
				let Some(material) = part.get_material() else {
					break 'mat_params;
				};
				let new_material = Material::copy(&material);
				part.material(&new_material);
				for (parameter_name, parameter_value) in material_parameters.drain() {
					parameter_value.apply_to_material(&client, &new_material, &parameter_name);
				}
			}
		}
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
	enabled: Arc<AtomicBool>,
	space: Arc<Spatial>,
	_resource_id: ResourceID,
	sk_model: OnceCell<SKModel>,
	parts: LifeLinkedNodeMap<i32>,
}

impl Model {
	pub fn add_to(node: &Arc<Node>, resource_id: ResourceID) -> Result<Arc<Model>> {
		let pending_model_path = get_resource_file(
			&resource_id,
			&*node.get_client().ok_or_else(|| eyre!("Client not found"))?,
			&[OsStr::new("glb"), OsStr::new("gltf")],
		)
		.ok_or_else(|| eyre!("Resource not found"))?;

		let model = Arc::new(Model {
			enabled: node.enabled.clone(),
			space: node.get_aspect::<Spatial>().unwrap().clone(),
			_resource_id: resource_id,
			sk_model: OnceCell::new(),
			parts: LifeLinkedNodeMap::default(),
		});
		MODEL_REGISTRY.add_raw(&model);

		// technically doing this in anything but the main thread isn't a good idea but dangit we need those model nodes ASAP
		let sk_model = SKModel::copy(SKModel::from_file(
			pending_model_path.to_str().unwrap(),
			None,
		)?);
		ModelPart::create_for_model(&model, &sk_model);
		let _ = model.sk_model.set(sk_model);
		node.add_aspect_raw(model.clone());
		Ok(model)
	}

	fn draw(&self, token: &MainThreadToken) {
		let Some(sk_model) = self.sk_model.get() else {
			return;
		};
		for model_node_node in self.parts.nodes() {
			if let Ok(model_node) = model_node_node.get_aspect::<ModelPart>() {
				model_node.update();
			};
		}

		if self.enabled.load(Ordering::Relaxed) {
			sk_model.draw(token, self.space.global_transform(), None, None);
		}
	}
}
// TODO: proper hread safety in stereokit_rust (probably just bind stereokit directly)
unsafe impl Send for Model {}
unsafe impl Sync for Model {}
impl Aspect for Model {
	const NAME: &'static str = "Model";
}
impl ModelAspect for Model {}
impl Drop for Model {
	fn drop(&mut self) {
		// if let Some(sk_model) = self.sk_model.take() {
		// destroy_queue::add(sk_model);
		// }
		MODEL_REGISTRY.remove(self);
	}
}

pub fn draw_all(token: &MainThreadToken) {
	for model in MODEL_REGISTRY.get_valid_contents() {
		model.draw(token);
	}
}

use super::{MODEL_PART_ASPECT_ALIAS_INFO, MaterialParameter, ModelAspect, ModelPartAspect};
use crate::bail;
use crate::core::client::Client;
use crate::core::error::Result;
use crate::core::registry::Registry;
use crate::core::resource::get_resource_file;
use crate::nodes::Node;
use crate::nodes::alias::{Alias, AliasList};
use crate::nodes::spatial::Spatial;
use color_eyre::eyre::eyre;
use glam::{Mat4, Vec2, Vec3};
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use stardust_xr::values::ResourceID;
use std::ffi::OsStr;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, LazyLock, OnceLock, Weak};
use stereokit_rust::material::Transparency;
use stereokit_rust::maths::Bounds;
use stereokit_rust::sk::MainThreadToken;
use stereokit_rust::{material::Material, model::Model as SKModel, tex::Tex, util::Color128};

pub struct MaterialWrapper(pub Material);
impl Drop for MaterialWrapper {
	fn drop(&mut self) {
		MATERIAL_REGISTRY.remove(self);
	}
}

impl Hash for MaterialWrapper {
	fn hash<H: Hasher>(&self, state: &mut H) {
		self.0.get_shader().0.as_ptr().hash(state);
		for param in self.0.get_all_param_info() {
			param.name.hash(state);
			(param.get_type() as u32).hash(state);
			let data = self
				.0
				.get_all_param_info()
				.get_data(&param.name, param.get_type());
			data.hash(state);
		}
		self.0.get_chain().map(MaterialWrapper).hash(state)
	}
}
impl PartialEq for MaterialWrapper {
	fn eq(&self, other: &Self) -> bool {
		if self.0.get_shader().0.as_ptr() != other.0.get_shader().0.as_ptr() {
			return false;
		}
		if self.0.get_all_param_info().count() != other.0.get_all_param_info().count() {
			return false;
		}
		for self_param in self.0.get_all_param_info() {
			let Some(other_param) = other
				.0
				.get_all_param_info()
				.get_data(self_param.get_name(), self_param.get_type())
			else {
				return false;
			};
			let Some(self_param) = self
				.0
				.get_all_param_info()
				.get_data(self_param.get_name(), self_param.get_type())
			else {
				return false;
			};
			if self_param != other_param {
				return false;
			}
		}
		self.0.get_chain().map(MaterialWrapper) == other.0.get_chain().map(MaterialWrapper)
	}
}
impl Eq for MaterialWrapper {}
unsafe impl Send for MaterialWrapper {}
unsafe impl Sync for MaterialWrapper {}

#[derive(Default)]
struct MaterialRegistry(Mutex<FxHashMap<u64, Weak<MaterialWrapper>>>);
impl MaterialRegistry {
	fn add_or_get(&self, material: Arc<MaterialWrapper>) -> Arc<MaterialWrapper> {
		let hash = {
			use std::hash::{Hash, Hasher};
			let mut hasher = std::collections::hash_map::DefaultHasher::new();
			material.hash(&mut hasher);
			hasher.finish()
		};

		let mut lock = self.0.lock();
		if let Some(mat) = lock.get(&hash) {
			if let Some(mat) = mat.upgrade() {
				return mat;
			}
		}

		lock.insert(hash, Arc::downgrade(&material));
		material
	}
	fn remove(&self, material: &MaterialWrapper) {
		let hash = {
			use std::hash::{Hash, Hasher};
			let mut hasher = std::collections::hash_map::DefaultHasher::new();
			material.hash(&mut hasher);
			hasher.finish()
		};
		let mut lock = self.0.lock();
		lock.remove(&hash);
	}
}

static MATERIAL_REGISTRY: LazyLock<MaterialRegistry> = LazyLock::new(MaterialRegistry::default);
static MODEL_REGISTRY: Registry<Model> = Registry::new();
static HOLDOUT_MATERIAL: OnceLock<Arc<MaterialWrapper>> = OnceLock::new();

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
				params.set_vector2(parameter_name, Vec2::from(*val));
			}
			MaterialParameter::Vec3(val) => {
				params.set_vector3(parameter_name, Vec3::from(*val));
			}
			MaterialParameter::Color(val) => {
				params.set_color(
					parameter_name,
					Color128::new(val.c.r, val.c.g, val.c.b, val.a),
				);
			}
			MaterialParameter::Texture(resource) => {
				let Some(texture_path) =
					get_resource_file(resource, client, &[OsStr::new("png"), OsStr::new("jpg")])
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
	path: String,
	space: Arc<Spatial>,
	model: Weak<Model>,
	material: Mutex<Option<Arc<MaterialWrapper>>>,
	pending_material_parameters: Mutex<FxHashMap<String, MaterialParameter>>,
	pending_material_replacement: Mutex<Option<Arc<MaterialWrapper>>>,
	aliases: AliasList,
}
impl ModelPart {
	fn create_for_model(model: &Arc<Model>, sk_model: &SKModel) {
		HOLDOUT_MATERIAL.get_or_init(|| {
			let mut mat = Material::copy(&Material::unlit());
			mat.transparency(Transparency::None);
			mat.color_tint(Color128::BLACK_TRANSPARENT);
			Arc::new(MaterialWrapper(mat))
		});

		let nodes = sk_model.get_nodes();
		for part in nodes.all() {
			ModelPart::create(model, &part);
		}
	}

	fn create(model: &Arc<Model>, part: &stereokit_rust::model::ModelNode) -> Option<Arc<Self>> {
		let mut parts = model.parts.lock();
		let parent_part = part
			.get_parent()
			.and_then(|part| parts.iter().find(|p| p.id == part.get_id()));

		let stardust_model_part = model.space.node()?;
		let client = stardust_model_part.get_client()?;
		let mut part_path = parent_part
			.map(|n| n.path.clone() + "/")
			.unwrap_or_default();
		part_path += part.get_name().unwrap();

		let node = client.scenegraph.add_node(Node::generate(&client, false));
		let spatial_parent = parent_part
			.map(|n| n.space.clone())
			.unwrap_or_else(|| model.space.clone());

		let local_transform = unsafe { part.get_local_transform().m };
		let space = Spatial::add_to(
			&node,
			Some(spatial_parent),
			Mat4::from_cols_array(&local_transform),
			false,
		);

		let _ = space.bounding_box_calc.set(|node| {
			let Ok(model_part) = node.get_aspect::<ModelPart>() else {
				return Bounds::default();
			};
			let Some(model) = model_part.model.upgrade() else {
				return Bounds::default();
			};
			let Some(sk_model) = model.sk_model.get() else {
				return Bounds::default();
			};
			let model_nodes = sk_model.get_nodes();
			let Some(model_node) = model_nodes.get_index(model_part.id) else {
				return Bounds::default();
			};
			let Some(sk_mesh) = model_node.get_mesh() else {
				return Bounds::default();
			};
			sk_mesh.get_bounds()
		});

		let model_part = Arc::new(ModelPart {
			id: part.get_id(),
			path: part_path,
			space,
			model: Arc::downgrade(model),
			pending_material_parameters: Mutex::new(FxHashMap::default()),
			pending_material_replacement: Mutex::new(None),
			aliases: AliasList::default(),
			material: Mutex::new(part.get_material().map(MaterialWrapper).map(Arc::new)),
		});
		node.add_aspect_raw(model_part.clone());
		parts.push(model_part.clone());
		Some(model_part)
	}

	pub fn replace_material(&self, replacement: Arc<MaterialWrapper>) {
		let shared_material = MATERIAL_REGISTRY.add_or_get(replacement);
		self.pending_material_replacement
			.lock()
			.replace(shared_material);
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
		let shared_material =
			MATERIAL_REGISTRY.add_or_get(Arc::new(MaterialWrapper(replacement.copy())));

		let mut lock = self.material.lock();
		part.material(&shared_material.0);
		lock.replace(shared_material);
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
			let mut lock = self.material.lock();
			part.material(&material_replacement.0);
			lock.replace(material_replacement);
		}

		'mat_params: {
			let mut material_parameters = self.pending_material_parameters.lock();
			if !material_parameters.is_empty() {
				let Some(material) = part.get_material() else {
					break 'mat_params;
				};
				let new_material = material.copy();
				for (parameter_name, parameter_value) in material_parameters.drain() {
					parameter_value.apply_to_material(&client, &new_material, &parameter_name);
				}

				let shared_material =
					MATERIAL_REGISTRY.add_or_get(Arc::new(MaterialWrapper(new_material)));
				let mut lock = self.material.lock();
				part.material(&shared_material.0);
				lock.replace(shared_material);
			}
		}
	}
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
	space: Arc<Spatial>,
	_resource_id: ResourceID,
	sk_model: OnceLock<SKModel>,
	parts: Mutex<Vec<Arc<ModelPart>>>,
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
			space: node.get_aspect::<Spatial>().unwrap().clone(),
			_resource_id: resource_id,
			sk_model: OnceLock::new(),
			parts: Mutex::new(Vec::default()),
		});
		MODEL_REGISTRY.add_raw(&model);

		// technically doing this in anything but the main thread isn't a good idea but dangit we need those model nodes ASAP
		let sk_model = SKModel::copy(&SKModel::from_file(
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
		let parts = self.parts.lock();
		for model_node in &*parts {
			model_node.update();
		}
		drop(parts);

		if let Some(node) = self.space.node() {
			if node.enabled() {
				sk_model.draw(token, self.space.global_transform(), None, None);
			}
		}
	}
}
// TODO: proper hread safety in stereokit_rust (probably just bind stereokit directly)
unsafe impl Send for Model {}
unsafe impl Sync for Model {}
impl ModelAspect for Model {
	#[doc = "Bind a model part to the node with the ID input."]
	fn bind_model_part(
		node: Arc<Node>,
		calling_client: Arc<Client>,
		id: u64,
		part_path: String,
	) -> Result<()> {
		let model = node.get_aspect::<Model>()?;
		let parts = model.parts.lock();
		let Some(part) = parts.iter().find(|p| p.path == part_path) else {
			let paths = parts.iter().map(|p| &p.path).collect::<Vec<_>>();
			bail!("Couldn't find model part at path {part_path}, all available paths: {paths:?}",);
		};
		Alias::create_with_id(
			&part.space.node().unwrap(),
			&calling_client,
			id,
			MODEL_PART_ASPECT_ALIAS_INFO.clone(),
			Some(&part.aliases),
		)?;
		Ok(())
	}
}
impl Drop for Model {
	fn drop(&mut self) {
		MODEL_REGISTRY.remove(self);
	}
}

pub fn draw_all(token: &MainThreadToken) {
	for model in MODEL_REGISTRY.get_valid_contents() {
		model.draw(token);
	}
}

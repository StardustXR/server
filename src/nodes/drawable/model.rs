use crate::bail;
use crate::bevy_plugin::MainWorldEntity;
use crate::core::client::Client;
use crate::core::error::Result;
use crate::core::registry::Registry;
use crate::core::resource::get_resource_file;
use crate::nodes::alias::{Alias, AliasList};
use crate::nodes::spatial::Spatial;
use crate::nodes::{Aspect, Node};
use crate::DefaultMaterial;
use bevy::app::{Plugin, PostUpdate};
use bevy::asset::Handle;
use bevy::asset::{AssetServer, Assets};
use bevy::color::{Alpha, Color, LinearRgba, Srgba};
use bevy::core::Name;
use bevy::gltf::GltfAssetLabel;
use bevy::math::bounding::Aabb3d;
use bevy::pbr::MeshMaterial3d;
use bevy::prelude::AlphaMode;
use bevy::prelude::{
	Children, Commands, Component, Deref, Entity, HierarchyQueryExt, Mesh3d, Parent, Query, Res,
	ResMut, Resource, Transform, Visibility, With, Without,
};
use bevy::reflect::{GetField, PartialReflect, Reflect};
use bevy::render::primitives::Aabb;
use bevy::scene::SceneRoot;
use color_eyre::eyre::eyre;
use glam::{Vec2, Vec3};
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use stardust_xr::values::ResourceID;
use tracing::{error, warn};

use std::ffi::OsStr;
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::{Arc, Weak};

static MODEL_REGISTRY: Registry<Model> = Registry::new();
static HOLDOUT_MATERIAL: OnceCell<Arc<DefaultMaterial>> = OnceCell::new();

impl MaterialParameter {
	fn apply_to_material(
		&self,
		client: &Client,
		material: &mut DefaultMaterial,
		parameter_name: &str,
	) {
		match self {
			MaterialParameter::Bool(val) => match parameter_name {
				name => {
					if let Some(field) = material.get_field_mut::<bool>(name) {
						*field = *val;
					} else {
						warn!("unknown bool material parameter name: {name}");
					}
				}
			},
			MaterialParameter::Int(val) => match parameter_name {
				name => {
					if let Some(field) = material.get_field_mut::<i32>(name) {
						*field = *val;
					} else {
						warn!("unknown bool material parameter name: {name}");
					}
				}
			},
			MaterialParameter::UInt(val) => match parameter_name {
				name => {
					if let Some(field) = material.get_field_mut::<u32>(name) {
						*field = *val;
					} else {
						warn!("unknown bool material parameter name: {name}");
					}
				}
			},
			MaterialParameter::Float(val) => match parameter_name {
				name => {
					if let Some(field) = material.get_field_mut::<f32>(name) {
						*field = *val;
					} else {
						warn!("unknown bool material parameter name: {name}");
					}
				}
			},
			MaterialParameter::Vec2(val) => match parameter_name {
				name => {
					if let Some(field) = material.get_field_mut::<Vec2>(name) {
						*field = (*val).into();
					} else {
						warn!("unknown bool material parameter name: {name}");
					}
				}
			},
			MaterialParameter::Vec3(val) => match parameter_name {
				name => {
					if let Some(field) = material.get_field_mut::<Vec3>(name) {
						*field = (*val).into();
					} else {
						warn!("unknown bool material parameter name: {name}");
					}
				}
			},
			MaterialParameter::Color(val) => match parameter_name {
				"color" => {
					material.base_color = LinearRgba::new(val.c.r, val.c.g, val.c.b, val.a).into()
				}
				name => {
					if let Some(field) = material.get_field_mut::<Color>(name) {
						*field = LinearRgba::new(val.c.r, val.c.g, val.c.b, val.a).into();
					} else {
						warn!("unknown bool material parameter name: {name}");
					}
				}
			},
			MaterialParameter::Texture(resource) => {
				match parameter_name {
					name => {
						warn!("unknown texture material parameter name: {name}");
					}
				}
				let Some(texture_path) =
					get_resource_file(resource, client, &[OsStr::new("png"), OsStr::new("jpg")])
				else {
					return;
				};
				error!("TODO: implement texture changing");
			}
		}
	}
}

pub struct ModelPart {
	entity: MainWorldEntity,
	path: String,
	space: Arc<Spatial>,
	model: Weak<Model>,
	pending_material_parameters: Mutex<FxHashMap<String, MaterialParameter>>,
	pending_material_replacement: Mutex<Option<Arc<DefaultMaterial>>>,
	aliases: AliasList,
}

#[derive(Component, Clone)]
pub struct StardustModel(Arc<Model>);
#[derive(Component, Clone)]
pub struct UnprocessedModel;
pub struct StardustModelPlugin;
impl Plugin for StardustModelPlugin {
	fn build(&self, app: &mut bevy::prelude::App) {
		let (tx, rx) = crossbeam_channel::unbounded();
		LOAD_MODEL_SENDER.set(tx);
		app.insert_resource(LoadModelReader(rx));
		app.add_systems(PostUpdate, create_model_parts_for_loaded_models);
	}
}
static LOAD_MODEL_SENDER: OnceCell<crossbeam_channel::Sender<(PathBuf, Arc<Model>)>> =
	OnceCell::new();
#[derive(Resource, Deref)]
struct LoadModelReader(crossbeam_channel::Receiver<(PathBuf, Arc<Model>)>);

fn update_models(mut query: Query<(&StardustModel, &mut Visibility, &mut Transform)>) {
	for (model, mut vis, mut transform) in query.iter_mut() {
		*transform = Transform::from_matrix(model.0.space.global_transform());
		if let Some(node) = model.0.space.node() {
			*vis = match node.enabled() {
				true => Visibility::Inherited,
				false => Visibility::Hidden,
			}
		}
	}
}

fn load_models(rx: Res<LoadModelReader>, mut cmds: Commands, asset_server: Res<AssetServer>) {
	for (path, model) in rx.try_iter() {
		let handle = asset_server.load(GltfAssetLabel::Scene(0).from_asset(path));
		let entity = cmds
			.spawn((
				SceneRoot(handle),
				StardustModel(model.clone()),
				UnprocessedModel,
			))
			.id();
		model.entity.set(entity);
	}
}

fn update_model_parts(
	models: Query<&StardustModel>,
	mut mats: ResMut<Assets<DefaultMaterial>>,
	mut part_query: Query<(
		&mut Transform,
		&mut MeshMaterial3d<DefaultMaterial>,
		&mut Mesh3d,
	)>,
) {
	for model in &models {
		let model = &model.0;
		for part in model.parts.lock().iter() {
			let Ok((mut transform, mut mat, mut _mesh)) = part_query.get_mut(*part.entity) else {
				continue;
			};
			*transform = Transform::from_matrix(Spatial::space_to_space_matrix(
				Some(&part.space),
				Some(&model.space),
			));
			if let Some(material_replacement) = part.pending_material_replacement.lock().take() {
				let material = material_replacement.deref().clone();
				let mat_handle = mats.add(material);
				mat.0 = mat_handle;
			}
			// todo: find all materials with identical parameters and batch them into 1 material again
			'mat_params: {
				let mut material_parameters = part.pending_material_parameters.lock();
				if !material_parameters.is_empty() {
					let Some(material) = mats.get(&mat.0) else {
						break 'mat_params;
					};

					let mut new_material = material.clone();
					let Some(client) = part.space.node().and_then(|v| v.get_client()) else {
						return;
					};
					for (parameter_name, parameter_value) in material_parameters.drain() {
						parameter_value.apply_to_material(
							&client,
							&mut new_material,
							&parameter_name,
						);
					}
					mat.0 = mats.add(new_material);
				}

				let shared_material =
					MATERIAL_REGISTRY.add_or_get(Arc::new(MaterialWrapper(new_material)));
				part.material(&shared_material.0);
			}
		}
	}
}

fn get_path(
	entity: Entity,
	query: &Query<(Entity, Option<&Parent>, &Transform, &Name, &Aabb), Without<SceneRoot>>,
) -> Option<String> {
	let (_, parent, _, name, _) = query.get(entity).ok()?;
	let next = parent.and_then(|p| get_path(p.get(), query));
	match next {
		Some(next) => Some(format!("{name}/{next}")),
		None => Some(name.to_string()),
	}
}

fn create_model_parts_for_loaded_models(
	query: Query<(Entity, &StardustModel), With<UnprocessedModel>>,
	children: Query<&Children>,
	gltf_model_parts: Query<
		(Entity, Option<&Parent>, &Transform, &Name, &Aabb),
		Without<SceneRoot>,
	>,
	mut cmds: Commands,
) {
	for (entity, model) in &query {
		let model = &model.0;
		let mut parts = model.parts.lock();
		cmds.entity(entity).remove::<UnprocessedModel>();
		for (entity, parent, transform, name, aabb) in children
			.iter_descendants(entity)
			.filter_map(|e| gltf_model_parts.get(e).ok())
		{
			let parent_part = parent
				.and_then(|e| gltf_model_parts.get(e.get()).ok())
				.and_then(|(e, _, _, _, _)| parts.iter().find(|v| v.entity.0 == e));

			let Some(stardust_model_part) = model.space.node() else {
				continue;
			};
			let Some(client) = stardust_model_part.get_client() else {
				continue;
			};
			let part_path = get_path(entity, &gltf_model_parts).unwrap_or_else(|| name.to_string());

			let node = client.scenegraph.add_node(Node::generate(&client, false));
			let spatial_parent = parent_part
				.map(|n| n.space.clone())
				.unwrap_or_else(|| model.space.clone());

			let space = Spatial::add_to(
				&node,
				Some(spatial_parent),
				transform.compute_matrix(),
				false,
			);

			*space.bounding_box_calc.lock() = Aabb3d::new(aabb.center, aabb.half_extents);

			let model_part = Arc::new(ModelPart {
				entity: MainWorldEntity(entity),
				path: part_path,
				space,
				model: Arc::downgrade(model),
				pending_material_parameters: Mutex::new(FxHashMap::default()),
				pending_material_replacement: Mutex::new(None),
				aliases: AliasList::default(),
			});
			<ModelPart as ModelPartAspect>::add_node_members(&node);
			node.add_aspect_raw(model_part.clone());
			parts.push(model_part.clone());
		}
	}
}

impl ModelPart {
	pub fn replace_material(&self, replacement: Arc<DefaultMaterial>) {
		self.pending_material_replacement
			.lock()
			.replace(replacement);
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
	entity: OnceCell<Entity>,
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
			entity: OnceCell::new(),
			parts: Mutex::new(Vec::default()),
		});
		MODEL_REGISTRY.add_raw(&model);
		if let Some(sender) = LOAD_MODEL_SENDER.get() {
			sender.send((pending_model_path, model.clone()));
		}
		node.add_aspect_raw(model.clone());
		Ok(model)
	}
}
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

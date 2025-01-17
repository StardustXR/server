use super::{MaterialParameter, ModelAspect, ModelPartAspect, MODEL_PART_ASPECT_ALIAS_INFO};
use crate::bevy_plugin::DESTROY_ENTITY;
use crate::core::client::Client;
use crate::core::error::{Result, ServerError};
use crate::core::registry::Registry;
use crate::core::resource::get_resource_file;
use crate::nodes::alias::{Alias, AliasList};
use crate::nodes::spatial::Spatial;
use crate::nodes::Node;
use crate::DefaultMaterial;
use bevy::app::{Plugin, PostUpdate, PreUpdate, Update};
use bevy::asset::{AssetServer, Assets, Handle};
use bevy::color::{Color, LinearRgba, Srgba};
use bevy::core::Name;
use bevy::gltf::GltfAssetLabel;
use bevy::image::Image;
use bevy::math::bounding::Aabb3d;
use bevy::pbr::MeshMaterial3d;
use bevy::prelude::{
	AlphaMode, BuildChildrenTransformExt, Children, Commands, Component, Deref, Entity, Has,
	HierarchyQueryExt, Parent, Query, Res, ResMut, Resource, Transform, Visibility, With, Without,
};
use bevy::reflect::GetField;
use bevy::render::primitives::Aabb;
use bevy::scene::SceneRoot;
use glam::{Mat4, Vec2, Vec3};
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use stardust_xr::values::ResourceID;
use tracing::{error, info, warn};

use std::ffi::OsStr;
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
		asset_server: &AssetServer,
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
						warn!("unknown i32 material parameter name: {name}");
					}
				}
			},
			MaterialParameter::UInt(val) => match parameter_name {
				name => {
					if let Some(field) = material.get_field_mut::<u32>(name) {
						*field = *val;
					} else {
						warn!("unknown u32 material parameter name: {name}");
					}
				}
			},
			MaterialParameter::Float(val) => match parameter_name {
				"cutoff" => {
					// should this only set the value if AlphaMode is already AlphaMode::Mask?
					material.alpha_mode = AlphaMode::Mask(*val);
				}
				"metallic" => {
					material.metallic = *val;
				}
				name => {
					if let Some(field) = material.get_field_mut::<f32>(name) {
						*field = *val;
					} else {
						warn!("unknown f32 material parameter name: {name}");
					}
				}
			},
			MaterialParameter::Vec2(val) => match parameter_name {
				name => {
					if let Some(field) = material.get_field_mut::<Vec2>(name) {
						*field = (*val).into();
					} else {
						warn!("unknown vec2 material parameter name: {name}");
					}
				}
			},
			MaterialParameter::Vec3(val) => match parameter_name {
				name => {
					if let Some(field) = material.get_field_mut::<Vec3>(name) {
						*field = (*val).into();
					} else {
						warn!("unknown vec3 material parameter name: {name}");
					}
				}
			},
			MaterialParameter::Color(val) => match parameter_name {
				"color" => {
					material.color = Srgba::new(val.c.r, val.c.g, val.c.b, val.a).into()
				}
				"emission_factor" => {
					material.emission_factor = LinearRgba::new(val.c.r, val.c.g, val.c.b, val.a).into()
				}
				name => {
					if let Some(field) = material.get_field_mut::<Color>(name) {
						*field = LinearRgba::new(val.c.r, val.c.g, val.c.b, val.a).into();
					} else {
						warn!("unknown color material parameter name: {name}");
					}
				}
			},
			MaterialParameter::Texture(resource) => {
				let Some(texture_path) =
					get_resource_file(resource, client, &[OsStr::new("png"), OsStr::new("jpg")])
				else {
					return;
				};
				let image = asset_server.load::<Image>(texture_path);
				match parameter_name {
					"diffuse" => {
						material.diffuse_texture.replace(image);
					}
					"emission" => {
						material.emission_texture.replace(image);
					}
					"normal" => {
						error!("TODO: implement Normal Map texture in bevy_sk");
						// material.n.replace(image);
					}
					"occlusion" => {
						material.occlusion_texture.replace(image);
					}
					// TODO: impl metalic and roughness textures, they are combined in bevy
					name => {
						if let Some(field) = material.get_field_mut::<Option<Handle<Image>>>(name) {
							field.replace(image);
						} else {
							warn!("unknown texture material parameter name: {name}");
						}
					}
				}
				error!("TODO: implement texture changing");
			}
		}
	}
}

pub struct ModelPart {
	entity: OnceCell<Entity>,
	path: String,
	space: Arc<Spatial>,
	model: Weak<Model>,
	pending_material_parameters: Mutex<FxHashMap<String, MaterialParameter>>,
	pending_material_replacement: Mutex<Option<Arc<DefaultMaterial>>>,
	aliases: AliasList,
}

#[derive(Component, Clone)]
pub struct StardustModel(Weak<Model>);
#[derive(Component, Clone)]
pub struct UnprocessedModel;
pub struct StardustModelPlugin;
impl Plugin for StardustModelPlugin {
	fn build(&self, app: &mut bevy::prelude::App) {
		let (tx, rx) = crossbeam_channel::unbounded();
		LOAD_MODEL_SENDER.set(tx);
		app.insert_resource(LoadModelReader(rx));
		app.add_systems(Update, create_model_parts_for_loaded_models);
		app.add_systems(PreUpdate, load_models);
		app.add_systems(PostUpdate, update_models);
		app.add_systems(PostUpdate, update_model_parts);
	}
}
static LOAD_MODEL_SENDER: OnceCell<crossbeam_channel::Sender<(PathBuf, Arc<Model>)>> =
	OnceCell::new();
#[derive(Resource, Deref)]
struct LoadModelReader(crossbeam_channel::Receiver<(PathBuf, Arc<Model>)>);

fn update_models(mut query: Query<(&StardustModel, &mut Visibility, &mut Transform)>) {
	for (model, mut vis, mut transform) in query.iter_mut() {
		let Some(model) = model.0.upgrade() else {
			continue;
		};
		*transform = Transform::from_matrix(model.space.global_transform());
		if let Some(node) = model.space.node() {
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
				StardustModel(Arc::downgrade(&model)),
				UnprocessedModel,
			))
			.id();
		model.entity.set(entity);
	}
}

fn update_model_parts(
	models: Query<&StardustModel, Without<UnprocessedModel>>,
	mut mats: ResMut<Assets<DefaultMaterial>>,
	mut part_query: Query<(
		&mut Transform,
		&mut MeshMaterial3d<DefaultMaterial>,
		&mut Visibility,
		Has<Parent>,
	)>,
	mut cmds: Commands,
	asset_server: Res<AssetServer>,
) {
	for model in &models {
		let Some(model) = model.0.upgrade() else {
			continue;
		};
		for part in model.parts.lock().iter() {
			let Some((entity, (mut transform, mut mat, mut vis, has_parent))) = part
				.entity
				.get()
				.and_then(|e| Some((*e, part_query.get_mut(*e).ok()?)))
			else {
				continue;
			};
			if has_parent {
				cmds.entity(entity).remove_parent_in_place();
			}
			*transform = Transform::from_matrix(part.space.global_transform());
			if let Some(node) = part.space.node() {
				*vis = match node.enabled() {
					true => Visibility::Inherited,
					false => Visibility::Hidden,
				}
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
							&asset_server,
						);
					}
					mat.0 = mats.add(new_material);
				}
			}
		}
	}
}

fn get_path(
	entity: Entity,
	query: &Query<(&Parent, &Name), Without<SceneRoot>>,
	mut in_vec: Vec<String>,
) -> Vec<String> {
	let Ok((parent, name)) = query.get(entity) else {
		return in_vec;
	};
	in_vec.push(name.to_string());
	get_path(parent.get(), query, in_vec)
}

fn create_model_parts_for_loaded_models(
	query: Query<(Entity, &StardustModel), (With<UnprocessedModel>, With<Children>)>,
	children: Query<&Children>,
	gltf_model_parts: Query<(Entity, &Transform, &Aabb), Without<SceneRoot>>,
	name_query: Query<(&Parent, &Name), Without<SceneRoot>>,
	mut cmds: Commands,
) {
	for (entity, model) in &query {
		info!("creating parts!");
		let Some(model) = model.0.upgrade() else {
			continue;
		};
		// let mut parts = model.parts.lock();
		let mut parts = Vec::<Arc<ModelPart>>::new();
		for (entity, transform, aabb) in children
			.iter_descendants_depth_first(entity)
			.filter_map(|e| gltf_model_parts.get(e).ok())
		{
			let mut path_parts = get_path(entity, &name_query, Vec::new());
			path_parts.remove(0);
			path_parts.reverse();
			let part_path = path_parts.join("/");
			path_parts.pop();
			let parent_path = path_parts.join("/");
			let parent_part = parts.iter().find(|v| v.path == parent_path);

			let Some(stardust_model_part) = model.space.node() else {
				continue;
			};
			let Some(client) = stardust_model_part.get_client() else {
				continue;
			};
			let model_part = model
				.parts
				.lock()
				.iter()
				.find(|v| v.path == part_path)
				.cloned()
				.map(|v| {
					*v.space.bounding_box_calc.lock() = Aabb3d::new(aabb.center, aabb.half_extents);
					if v.entity.set(entity).is_err() {
						error!(
							"trying to set entity for already init model part?!
							please yell at schmarni if you see this"
						);
					};

					if let Err(err) = v.space.set_spatial_parent(Some(
						parent_part.map(|n| &n.space).unwrap_or_else(|| {
							info!("model is spatial parent");
							&model.space
						}),
					)) {
						error!("error setting spatial parent for existing model part: {err}");
					}
					v.space.set_local_transform(transform.compute_matrix());
					info!("not fresh {}", &v.path);
					v
				})
				.unwrap_or_else(|| {
					let node = client.scenegraph.add_node(Node::generate(&client, false));
					let spatial_parent =
						parent_part.map(|n| n.space.clone()).unwrap_or_else(|| {
							info!("model is spatial parent");
							model.space.clone()
						});

					let space = Spatial::add_to(
						&node,
						Some(spatial_parent),
						transform.compute_matrix(),
						false,
					);

					*space.bounding_box_calc.lock() = Aabb3d::new(aabb.center, aabb.half_extents);

					let model_part = Arc::new(ModelPart {
						entity: OnceCell::from(entity),
						path: part_path,
						space,
						model: Arc::downgrade(&model),
						pending_material_parameters: Mutex::new(FxHashMap::default()),
						pending_material_replacement: Mutex::new(None),
						aliases: AliasList::default(),
					});
					node.add_aspect_raw(model_part.clone());
					info!("fresh {}", &model_part.path);
					model_part
				});
			parts.push(model_part.clone());
		}
		cmds.entity(entity).remove::<UnprocessedModel>();
		info!("created parts! {}", parts.len());
		*model.parts.lock() = parts;
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
			&*node.get_client().ok_or(ServerError::NoClient)?,
			&[OsStr::new("glb"), OsStr::new("gltf")],
		)
		.ok_or(ServerError::NoResource)?;

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
		let mut parts = model.parts.lock();
		let part =
			parts
				.iter()
				.find(|p| p.path == part_path)
				.cloned()
				.unwrap_or_else(|| {
					let paths = parts.iter().map(|p| &p.path).collect::<Vec<_>>();
					error!("Couldn't find model part at path {part_path}, all available paths: {paths:?}");

					let node = calling_client
						.scenegraph
						.add_node(Node::generate(&calling_client, false));

					let space = Spatial::add_to(&node, None, Mat4::IDENTITY, false);

					let model_part = Arc::new(ModelPart {
						entity: OnceCell::new(),
						path: part_path,
						space,
						model: Arc::downgrade(&model),
						pending_material_parameters: Mutex::new(FxHashMap::default()),
						pending_material_replacement: Mutex::new(None),
						aliases: AliasList::default(),
					});
					node.add_aspect_raw(model_part.clone());
					parts.push(model_part.clone());
					model_part
				});
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
impl Drop for ModelPart {
	fn drop(&mut self) {
		if let Some(e) = self.entity.get() {
			_ = DESTROY_ENTITY.send(*e);
		}
	}
}
impl Drop for Model {
	fn drop(&mut self) {
		if let Some(e) = self.entity.get() {
			_ = DESTROY_ENTITY.send(*e);
		}
		MODEL_REGISTRY.remove(self);
	}
}

use super::{MODEL_PART_ASPECT_ALIAS_INFO, MaterialParameter, ModelAspect, ModelPartAspect};
use crate::core::bevy_channel::{BevyChannel, BevyChannelReader};
use crate::core::client::Client;
use crate::core::color::ColorConvert as _;
use crate::core::entity_handle::EntityHandle;
use crate::core::error::Result;
use crate::core::registry::Registry;
use crate::core::resource::get_resource_file;
use crate::nodes::Node;
use crate::nodes::alias::{Alias, AliasList};
use crate::nodes::spatial::{Spatial, SpatialNode};
use crate::{BevyMaterial, bail};
use bevy::asset::{load_internal_asset, weak_handle};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::primitives::Aabb;
use bevy::render::render_resource::{AsBindGroup, ShaderRef};
use color_eyre::eyre::eyre;
use parking_lot::Mutex;
use rustc_hash::{FxHashMap, FxHasher};
use stardust_xr::values::ResourceID;
use std::ffi::OsStr;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, Weak};

static LOAD_MODEL: BevyChannel<(Arc<Model>, PathBuf)> = BevyChannel::new();

type HoldoutMaterial = ExtendedMaterial<BevyMaterial, HoldoutExtension>;
const HOLDOUT_SHADER_HANDLE: Handle<Shader> = weak_handle!("92b481b7-d3da-4188-b252-2335ec814ee2");
const HOLDOUT_MATERIAL_HANDLE: Handle<HoldoutMaterial> =
	weak_handle!("d56f1d62-9121-434b-a34f-9f0bbd6b3390");

pub struct ModelNodePlugin;
impl Plugin for ModelNodePlugin {
	fn build(&self, app: &mut App) {
		LOAD_MODEL.init(app);

		load_internal_asset!(
			app,
			HOLDOUT_SHADER_HANDLE,
			"holdout.wgsl",
			Shader::from_wgsl
		);
		app.add_plugins(MaterialPlugin::<HoldoutMaterial>::default());
		app.world_mut()
			.resource_mut::<Assets<HoldoutMaterial>>()
			.insert(&HOLDOUT_MATERIAL_HANDLE, HoldoutMaterial::default());

		app.init_resource::<MaterialRegistry>();
		app.add_systems(Update, load_models);
		app.add_systems(
			PostUpdate,
			(
				gen_model_parts.after(TransformSystem::TransformPropagate),
				apply_materials,
			)
				.chain(),
		);
	}
}

// No extra data needed for a simple holdout
#[derive(Default, Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct HoldoutExtension {}
impl MaterialExtension for HoldoutExtension {
	fn fragment_shader() -> ShaderRef {
		HOLDOUT_SHADER_HANDLE.into()
	}

	fn alpha_mode() -> Option<AlphaMode> {
		Some(AlphaMode::Opaque)
	}
}

#[derive(Component)]
struct ModelNode(Weak<Model>);

fn load_models(
	asset_server: Res<AssetServer>,
	mut cmds: Commands,
	mut mpsc_receiver: ResMut<BevyChannelReader<(Arc<Model>, PathBuf)>>,
) {
	while let Some((model, path)) = mpsc_receiver.read() {
		// idk of the asset label is the correct approach here
		let handle = asset_server.load(GltfAssetLabel::Scene(0).from_asset(path));
		let entity = cmds
			.spawn((
				Name::new("ModelNode"),
				SceneRoot(handle),
				ModelNode(Arc::downgrade(&model)),
				SpatialNode(Arc::downgrade(&model.spatial)),
			))
			.id();
		model.bevy_scene_entity.set(entity.into()).unwrap();
	}
}

fn apply_materials(
	mut commands: Commands,
	mut query: Query<&mut MeshMaterial3d<BevyMaterial>>,
	mut material_registry: ResMut<MaterialRegistry>,
	asset_server: Res<AssetServer>,
	mut materials: ResMut<Assets<BevyMaterial>>,
) -> bevy::prelude::Result {
	for model_part in MODEL_REGISTRY
		.get_valid_contents()
		.iter()
		.filter_map(|p| p.parts.get())
		.flatten()
	{
		let entity = **model_part.mesh_entity.get().unwrap();
		let Ok(mut mesh_mat) = query.get_mut(entity) else {
			continue;
		};
		if model_part.holdout.load(Ordering::Relaxed) {
			commands
				.entity(entity)
				.remove::<MeshMaterial3d<BevyMaterial>>()
				.insert(MeshMaterial3d(HOLDOUT_MATERIAL_HANDLE));
			continue;
		}
		if let Some(material) = model_part.pending_material_replacement.lock().take()
			&& let Some(material) = materials.get(&material)
		{
			let handle = material_registry.get_handle(material.clone(), &mut materials);
			mesh_mat.0 = handle;
		}
		for (param_name, param) in model_part.pending_material_parameters.lock().drain() {
			let mut new_mat = materials.get(&mesh_mat.0).unwrap().clone();
			param.apply_to_material(
				&model_part.space.node().unwrap().get_client().unwrap(),
				&mut new_mat,
				&param_name,
				&asset_server,
			);
			let handle = material_registry.get_handle(new_mat, &mut materials);
			mesh_mat.0 = handle;
		}
	}

	Ok(())
}

fn gen_model_parts(
	scenes: Res<Assets<Scene>>,
	query: Query<(&SceneRoot, &ModelNode, &Children)>,
	children_query: Query<&Children>,
	part_query: Query<(&Name, Option<&Children>, &Transform), Without<Mesh3d>>,
	part_mesh_query: Query<(&Transform, &Aabb), With<Mesh3d>>,
	global_transform_query: Query<&GlobalTransform>,
	has_mesh: Query<Has<Mesh3d>>,
	mut cmds: Commands,
) {
	for (scene_root, model_node, model_children) in query.iter() {
		let Some(model) = model_node.0.upgrade() else {
			continue;
		};
		if model.parts.get().is_some() {
			continue;
		}
		if scenes.get(scene_root.0.id()).is_none() {
			continue;
		}
		let mut parts = Vec::new();
		for entity in model_children
			.iter()
			.filter_map(|e| children_query.get(e).ok())
			.flat_map(|c| c.iter())
		{
			gen_path(
				entity,
				&part_query,
				None,
				&mut |entity, name, transform, parent, children| {
					let path = parent
						.as_ref()
						.map(|p| format!("{}/{}", &p.path, name.as_str()))
						.unwrap_or_else(|| name.to_string());
					let parent_spatial = parent
						.as_ref()
						.map(|p| p.space.clone())
						.unwrap_or_else(|| model.spatial.clone());
					let client = model.spatial.node()?.get_client()?;
					let (spatial, model_part) =
						match model.pre_bound_parts.lock().iter().find(|v| v.path == path) {
							None => {
								let node =
									client.scenegraph.add_node(Node::generate(&client, false));
								let spatial = Spatial::add_to(
									&node,
									Some(parent_spatial),
									transform.compute_matrix(),
									false,
								);
								let model_part = node.add_aspect(ModelPart {
									entity: OnceLock::new(),
									mesh_entity: OnceLock::new(),
									path,
									space: spatial.clone(),
									_model: Arc::downgrade(&model),
									pending_material_parameters: Mutex::default(),
									pending_material_replacement: Mutex::default(),
									holdout: AtomicBool::new(false),
									aliases: AliasList::default(),
									bounds: OnceLock::new(),
								});
								(spatial, model_part)
							}
							Some(part) => {
								part.space.set_spatial_parent(&parent_spatial).unwrap();
								(part.space.clone(), part.clone())
							}
						};
					let aabb = Aabb::enclosing(
						children
							.iter()
							.flat_map(|v| v.iter())
							.filter_map(|e| part_mesh_query.get(e).ok())
							.flat_map(|(transform, aabb)| {
								[
									transform.transform_point(aabb.min().into()),
									transform.transform_point(aabb.max().into()),
								]
							}),
					)
					.unwrap_or_default();
					_ = spatial.bounding_box_calc.set(move |n| {
						n.get_aspect::<ModelPart>()
							.ok()
							.and_then(|v| v.bounds.get().copied())
							.unwrap_or_default()
					});
					spatial.set_local_transform(transform.compute_matrix());

					cmds.entity(entity)
						.insert(SpatialNode(Arc::downgrade(&spatial)));
					let mesh_entity = children_query
						.get(entity)
						.iter()
						.flat_map(|v| v.iter())
						.find(|e| has_mesh.get(*e).unwrap_or(false))?;
					_ = model_part.bounds.set(aabb);
					_ = model_part.entity.set(entity.into());
					_ = model_part.mesh_entity.set(mesh_entity.into());
					parts.push(model_part.clone());
					Some(model_part)
				},
			);
		}
		_ = model.parts.set(parts);
	}
}

fn gen_path(
	current_entity: Entity,
	part_query: &Query<(&Name, Option<&Children>, &Transform), Without<Mesh3d>>,
	parent: Option<Arc<ModelPart>>,
	func: &mut dyn FnMut(
		Entity,
		&Name,
		&Transform,
		Option<Arc<ModelPart>>,
		Option<&Children>,
	) -> Option<Arc<ModelPart>>,
) {
	let Ok((name, children, transform)) = part_query.get(current_entity) else {
		return;
	};
	let Some(parent) = func(current_entity, name, transform, parent, children) else {
		return;
	};
	for e in children.iter().flat_map(|c| c.iter()) {
		gen_path(e, part_query, Some(parent.clone()), func);
	}
}

#[derive(PartialEq, Deref, DerefMut, Clone, Copy, Eq, PartialOrd, Ord, Hash)]
struct HashedPbrMaterial(u64);
impl HashedPbrMaterial {
	fn new(material: &BevyMaterial) -> Self {
		let mut hasher = FxHasher::default();
		Self::hash_pbr_mat(material, &mut hasher);
		Self(hasher.finish())
	}
	fn hash_pbr_mat<H: Hasher>(mat: &BevyMaterial, state: &mut H) {
		hash_color(mat.base_color, state);
		hash_color(mat.emissive.into(), state);
		state.write_u32(mat.metallic.to_bits());
		state.write_u32(mat.perceptual_roughness.to_bits());
		match mat.alpha_mode {
			AlphaMode::Opaque => state.write_u8(0),
			AlphaMode::Mask(v) => {
				state.write_u8(1);
				state.write_u32(v.to_bits());
			}
			AlphaMode::Blend => state.write_u8(2),
			AlphaMode::Premultiplied => state.write_u8(3),
			AlphaMode::AlphaToCoverage => state.write_u8(4),
			AlphaMode::Add => state.write_u8(5),
			AlphaMode::Multiply => state.write_u8(6),
		}
		state.write_u8(mat.double_sided as u8);
		mat.base_color_texture.hash(state);
		mat.emissive_texture.hash(state);
		mat.metallic_roughness_texture.hash(state);
		mat.occlusion_texture.hash(state);
		// should always be the same, TODO: make the spherical harmonics buffer a per mesh instance thing
		// mat.spherical_harmonics.hash(state);
	}
}
fn hash_color<H: Hasher>(color: Color, state: &mut H) {
	match color {
		Color::Srgba(srgba) => {
			state.write_u8(0);
			state.write(&srgba.to_u8_array());
		}
		Color::LinearRgba(linear_rgba) => {
			state.write_u8(1);
			state.write(&linear_rgba.to_u8_array());
		}
		Color::Hsla(hsla) => {
			state.write_u8(2);
			hsla.to_f32_array()
				.iter()
				.for_each(|v| state.write_u32(v.to_bits()));
		}
		Color::Hsva(hsva) => {
			state.write_u8(3);
			hsva.to_f32_array()
				.iter()
				.for_each(|v| state.write_u32(v.to_bits()));
		}
		Color::Hwba(hwba) => {
			state.write_u8(4);
			hwba.to_f32_array()
				.iter()
				.for_each(|v| state.write_u32(v.to_bits()));
		}
		Color::Laba(laba) => {
			state.write_u8(5);
			laba.to_f32_array()
				.iter()
				.for_each(|v| state.write_u32(v.to_bits()));
		}
		Color::Lcha(lcha) => {
			state.write_u8(6);
			lcha.to_f32_array()
				.iter()
				.for_each(|v| state.write_u32(v.to_bits()));
		}
		Color::Oklaba(oklaba) => {
			state.write_u8(7);
			oklaba
				.to_f32_array()
				.iter()
				.for_each(|v| state.write_u32(v.to_bits()));
		}
		Color::Oklcha(oklcha) => {
			state.write_u8(8);
			oklcha
				.to_f32_array()
				.iter()
				.for_each(|v| state.write_u32(v.to_bits()));
		}
		Color::Xyza(xyza) => {
			state.write_u8(9);
			xyza.to_f32_array()
				.iter()
				.for_each(|v| state.write_u32(v.to_bits()));
		}
	}
}
static MODEL_REGISTRY: Registry<Model> = Registry::new();

impl MaterialParameter {
	fn apply_to_material(
		&self,
		client: &Client,
		mat: &mut BevyMaterial,
		parameter_name: &str,
		asset_server: &AssetServer,
	) {
		match self {
			MaterialParameter::Bool(val) => match parameter_name {
				"double_sided" => mat.double_sided = *val,
				v => {
					error!("unknown param_name ({v}) for color")
				}
			},
			MaterialParameter::Int(_val) => {
				// nothing uses an int
			}
			MaterialParameter::UInt(_val) => {
				// nothing uses an uint
			}
			MaterialParameter::Float(val) => {
				match parameter_name {
					"metallic" => mat.metallic = *val,
					"roughness" => mat.perceptual_roughness = *val,
					// we probably don't want to expose tex_scale
					// "tex_scale" => mat.tex_scale = *val,
					v => {
						error!("unknown param_name ({v}) for float")
					}
				}
			}
			MaterialParameter::Vec2(_val) => {
				// nothing uses a Vec2
			}
			MaterialParameter::Vec3(_val) => {
				// nothing uses a Vec3
			}
			MaterialParameter::Color(color) => match parameter_name {
				"color" => mat.base_color = color.to_bevy(),
				"emission_factor" => mat.emissive = color.to_bevy().to_linear(),
				v => {
					error!("unknown param_name ({v}) for color")
				}
			},
			MaterialParameter::Texture(resource) => {
				let Some(texture_path) =
					get_resource_file(resource, client, &[OsStr::new("png"), OsStr::new("jpg")])
				else {
					return;
				};
				let handle = asset_server.load(texture_path);
				match parameter_name {
					"diffuse" => mat.base_color_texture = Some(handle),
					"emission" => mat.emissive_texture = Some(handle),
					"metal" => mat.metallic_roughness_texture = Some(handle),
					"occlusion" => mat.occlusion_texture = Some(handle),
					v => {
						error!("unknown param_name ({v}) for texture");
					}
				}
				// mat.alpha_mode = AlphaMode::Blend;
			}
		}
	}
}

pub struct ModelPart {
	entity: OnceLock<EntityHandle>,
	mesh_entity: OnceLock<EntityHandle>,
	path: String,
	space: Arc<Spatial>,
	_model: Weak<Model>,
	pending_material_parameters: Mutex<FxHashMap<String, MaterialParameter>>,
	pending_material_replacement: Mutex<Option<Handle<BevyMaterial>>>,
	holdout: AtomicBool,
	aliases: AliasList,
	bounds: OnceLock<Aabb>,
}
impl ModelPart {
	pub fn replace_material(&self, replacement: Handle<BevyMaterial>) {
		self.pending_material_replacement
			.lock()
			.replace(replacement);
	}
	pub fn set_material_parameter(&self, parameter_name: String, value: MaterialParameter) {
		self.pending_material_parameters
			.lock()
			.insert(parameter_name, value);
	}
}
impl ModelPartAspect for ModelPart {
	#[doc = "Set this model part's material to one that cuts a hole in the world. Often used for overlays/passthrough where you want to show the background through an object."]
	fn apply_holdout_material(node: Arc<Node>, _calling_client: Arc<Client>) -> Result<()> {
		let model_part = node.get_aspect::<ModelPart>()?;
		model_part.holdout.store(true, Ordering::Relaxed);
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
		model_part.set_material_parameter(parameter_name, value);
		Ok(())
	}
}
#[derive(Default, Resource)]
pub struct MaterialRegistry(FxHashMap<HashedPbrMaterial, Handle<BevyMaterial>>);
impl MaterialRegistry {
	/// returns strong handle for PbrMaterial elminitating duplications
	pub fn get_handle(
		&mut self,
		material: BevyMaterial,
		materials: &mut ResMut<Assets<BevyMaterial>>,
	) -> Handle<BevyMaterial> {
		let hash = HashedPbrMaterial::new(&material);
		match self
			.0
			.get(&hash)
			.and_then(|v| materials.get_strong_handle(v.id()))
		{
			Some(v) => v,
			None => {
				let handle = materials.add(material);
				self.0.insert(hash, handle.clone_weak());
				handle
			}
		}
	}
}

pub struct Model {
	spatial: Arc<Spatial>,
	_resource_id: ResourceID,
	bevy_scene_entity: OnceLock<EntityHandle>,
	parts: OnceLock<Vec<Arc<ModelPart>>>,
	pre_bound_parts: Mutex<Vec<Arc<ModelPart>>>,
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
			spatial: node.get_aspect::<Spatial>().unwrap().clone(),
			_resource_id: resource_id,
			bevy_scene_entity: OnceLock::new(),
			pre_bound_parts: Mutex::default(),
			parts: OnceLock::new(),
		});
		LOAD_MODEL
			.send((model.clone(), pending_model_path))
			.unwrap();
		MODEL_REGISTRY.add_raw(&model);

		node.add_aspect_raw(model.clone());
		Ok(model)
	}
	pub fn get_model_part(self: &Arc<Self>, part_path: String) -> Result<Arc<ModelPart>> {
		let part = match self
			.parts
			.get()
			.map(|v| v.iter().find(|p| p.path == part_path))
		{
			Some(Some(part)) => part.clone(),
			Some(None) => {
				let paths = self
					.parts
					.get()
					.unwrap()
					.iter()
					.map(|p| &p.path)
					.collect::<Vec<_>>();
				bail!(
					"Couldn't find model part at path {part_path}, all available paths: {paths:?}",
				);
			}
			None => {
				// TODO: this could be a denail of service vector
				let client = self.spatial.node().unwrap().get_client().unwrap();
				let part_node = client.scenegraph.add_node(Node::generate(&client, false));
				let spatial = Spatial::add_to(
					&part_node,
					Some(self.spatial.clone()),
					Mat4::IDENTITY,
					false,
				);
				let part = part_node.add_aspect(ModelPart {
					entity: OnceLock::new(),
					mesh_entity: OnceLock::new(),
					path: part_path,
					space: spatial,
					_model: Arc::downgrade(self),
					pending_material_parameters: Mutex::default(),
					pending_material_replacement: Mutex::default(),
					holdout: AtomicBool::new(false),
					aliases: AliasList::default(),
					bounds: OnceLock::new(),
				});
				self.pre_bound_parts.lock().push(part.clone());
				part
			}
		};
		Ok(part)
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
		let part = model.get_model_part(part_path)?;
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

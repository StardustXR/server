use crate::{
	BevyMaterial, PION,
	bevy_int::{
		bevy_channel::{BevyChannel, BevyChannelReader},
		color::ColorConvert as _,
		entity_handle::EntityHandle,
	},
	core::{error::Result, registry::Registry, resource::get_resource_file},
	impl_proxy, impl_transaction_handler, interface,
	nodes::{
		ProxyExt,
		drawable::{
			ModelNodeSystemSet,
			dmatex::{Dmatex, DmatexExt as _, SignalOnDrop},
		},
		ref_owned,
		spatial::{BoundingBoxCalc, Spatial, SpatialNode, SpatialObject},
	},
};
use bevy::{
	asset::{load_internal_asset, weak_handle},
	gltf::GltfLoaderSettings,
	pbr::{ExtendedMaterial, MaterialExtension},
	prelude::*,
	render::{
		Render, RenderApp, RenderSet,
		primitives::Aabb,
		render_resource::{AsBindGroup, ShaderRef},
	},
};
use binderbinder::binder_object::BinderObject;
use color_eyre::eyre::eyre;
use gluon_wire::{GluonCtx, drop_tracking::DropNotifier};
use parking_lot::Mutex;
use rustc_hash::{FxHashMap, FxHasher};
use stardust_xr_protocol::{
	model::{
		MaterialParamError, MaterialParameter, Model as ModelProxy, ModelHandler,
		ModelInterfaceHandler, ModelPart as ModelPartProxy, ModelPartHandler, NonUniformTransform,
		PartialNonUniformTransform,
	},
	spatial::Spatial as SpatialProxy,
	types::{Resource, Vec3F},
};
use stardust_xr_server_foundation::on_drop::AbortOnDrop;
use std::{
	collections::{HashMap, VecDeque},
	ffi::OsStr,
	hash::{Hash, Hasher},
	path::PathBuf,
	str::FromStr,
	sync::{
		Arc, OnceLock, Weak,
		atomic::{AtomicBool, Ordering},
	},
};
use tokio::sync::{Notify, RwLock, oneshot};
use vulkano::{VulkanObject, sync::semaphore::Semaphore};
use wgpu_hal::vulkan::WAIT_SEMAPHORES;

static LOAD_MODEL: BevyChannel<(Arc<BinderObject<Model>>, PathBuf)> = BevyChannel::new();

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
		app.add_systems(
			Update,
			(load_models, gen_model_parts, apply_materials)
				.chain()
				.in_set(ModelNodeSystemSet),
		);

		app.sub_app_mut(RenderApp)
			.add_systems(Render, queue_semaphores.in_set(RenderSet::Prepare))
			.add_systems(Render, drop_semaphores.in_set(RenderSet::Cleanup));
	}
}

fn queue_semaphores() {
	WAIT_SEMAPHORES
		.lock()
		.extend(ACQUIRE_SEMAPHORES.lock().iter().map(|v| v.handle()));
}
fn drop_semaphores() {
	ACQUIRE_SEMAPHORES.lock().drain(..);
}

// No extra data needed for a simple holdout
#[derive(Default, Asset, AsBindGroup, TypePath, Debug, Clone)]
#[data(50, u32, binding_array(101))]
#[bindless(index_table(range(50..51), binding(100)))]
pub struct HoldoutExtension {}
impl From<&HoldoutExtension> for u32 {
	fn from(_: &HoldoutExtension) -> Self {
		0
	}
}
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
		let handle = asset_server.load_with_settings(
			GltfAssetLabel::Scene(0).from_asset(path),
			|settings: &mut GltfLoaderSettings| {
				settings.load_cameras = false;
				settings.load_lights = false;
			},
		);
		let entity = cmds
			.spawn((
				Name::new("ModelNode"),
				SceneRoot(handle),
				ModelNode(Arc::downgrade(&model)),
				SpatialNode(Arc::downgrade(&model.spatial)),
				Visibility::Hidden,
			))
			.id();
		model
			.bevy_scene_entity
			.set(EntityHandle::new(entity))
			.unwrap();
	}
}

fn apply_materials(
	mut commands: Commands,
	mut query: Query<&mut MeshMaterial3d<BevyMaterial>>,
	mut material_registry: ResMut<MaterialRegistry>,
	asset_server: Res<AssetServer>,
	mut materials: ResMut<Assets<BevyMaterial>>,
) -> bevy::prelude::Result {
	for (model, model_part) in MODEL_REGISTRY
		.get_valid_contents()
		.iter()
		.filter_map(|p| Some(p.parts.get()?.iter().map(move |v| (p, v))))
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
			apply_to_material(
				&param,
				&model.resource_prefixes,
				&mut new_mat,
				&mut model_part.textures.lock(),
				&param_name,
				&asset_server,
			);
			let handle = material_registry.get_handle(new_mat, &mut materials);
			mesh_mat.0 = handle;
		}
		for (slot, queue) in model_part.pending_dmatexes.lock().iter_mut() {
			while let Some((mut recv, _)) = if queue.front().is_some_and(|v| !v.0.is_empty()) {
				queue.pop_front()
			} else {
				None
			} {
				let Ok((release_signal, tex)) = recv.try_recv() else {
					error!("somehow the oneshot channel wasn't empty but also failed to try_recv");
					continue;
				};
				// TODO: handle bevy handles possibly not existing yet
				let Some(handle) = tex.try_get_bevy_handle() else {
					error!("tried to apply dmatex before its bevy handle was created");
					continue;
				};
				slot.get_part_texture(&mut model_part.textures.lock())
					.replace((handle, Some(release_signal)));
			}
		}
		{
			let tex = model_part.textures.lock();
			let mut new_mat = materials.get(&mesh_mat.0).unwrap().clone();
			if let Some(tex) = &tex.diffuse {
				new_mat.base_color_texture = Some(tex.0.clone());
			}
			if let Some(tex) = &tex.emission {
				new_mat.emissive_texture = Some(tex.0.clone());
			}
			if let Some(tex) = &tex.metal {
				new_mat.metallic_roughness_texture = Some(tex.0.clone());
			}
			if let Some(tex) = &tex.occlusion {
				new_mat.occlusion_texture = Some(tex.0.clone());
			}
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
						.map(|p| p.spatial.clone())
						.unwrap_or_else(|| model.spatial.clone());
					let spatial =
						SpatialObject::new(Some(&parent_spatial), transform.compute_matrix());
					let model_part = PION.register_object(ModelPart {
						entity: OnceLock::new(),
						mesh_entity: OnceLock::new(),
						path,
						spatial: spatial.clone(),
						pending_material_parameters: Mutex::default(),
						pending_material_replacement: Mutex::default(),
						holdout: AtomicBool::new(false),
						bounds: OnceLock::new(),
						pending_dmatexes: Mutex::default(),
						textures: Mutex::default(),
						bounding_calc: OnceLock::new(),
						drop_notifs: RwLock::default(),
					});
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
					let weak_part = Arc::downgrade(&model_part);
					let calc = spatial.custom_bounding_box(move || {
						weak_part
							.upgrade()
							.and_then(|v| v.bounds.get().copied())
							.unwrap_or_default()
					});
					_ = model_part.bounding_calc.set(calc);
					let _ = spatial.set_spatial_parent(&parent_spatial);
					spatial.set_local_transform(transform.compute_matrix());
					let entity_handle = EntityHandle::new(entity);
					spatial.set_entity(entity_handle.clone());
					cmds.entity(entity)
						.insert(SpatialNode(Arc::downgrade(&spatial)));
					let mesh_entity = children_query
						.get(entity)
						.iter()
						.flat_map(|v| v.iter())
						.find(|e| has_mesh.get(*e).unwrap_or(false))?;
					_ = model_part.bounds.set(aabb);
					_ = model_part.entity.set(entity_handle);
					_ = model_part.mesh_entity.set(EntityHandle::new(mesh_entity));
					parts.push(model_part.clone());
					Some(model_part)
				},
			);
		}
		_ = model.parts.set(parts);
		model
			.spatial
			.set_entity(model.bevy_scene_entity.get().unwrap().clone());
		model.setup_complete_notify.notify_waiters();
	}
}

fn gen_path(
	current_entity: Entity,
	part_query: &Query<(&Name, Option<&Children>, &Transform), Without<Mesh3d>>,
	parent: Option<Arc<BinderObject<ModelPart>>>,
	func: &mut dyn FnMut(
		Entity,
		&Name,
		&Transform,
		Option<Arc<BinderObject<ModelPart>>>,
		Option<&Children>,
	) -> Option<Arc<BinderObject<ModelPart>>>,
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
		state.write_u8(mat.unlit as u8);
		mat.base_color_texture.hash(state);
		mat.emissive_texture.hash(state);
		mat.metallic_roughness_texture.hash(state);
		mat.occlusion_texture.hash(state);
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
static MODEL_REGISTRY: Registry<BinderObject<Model>> = Registry::new();

fn apply_to_material(
	param: &MaterialParameter,
	resource_prefixes: &[PathBuf],
	mat: &mut BevyMaterial,
	part_textures: &mut PartTextures,
	parameter_name: &str,
	asset_server: &AssetServer,
) {
	match param {
		MaterialParameter::Bool { value } => match parameter_name {
			"double_sided" => mat.double_sided = *value,
			"unlit" => mat.unlit = *value,
			"opaque" => {
				mat.alpha_mode = if *value {
					AlphaMode::Opaque
				} else {
					AlphaMode::Premultiplied
				}
			}

			v => {
				error!("unknown param_name ({v}) for color")
			}
		},
		MaterialParameter::Int { value: _ } => {
			// nothing uses an int
		}
		MaterialParameter::Uint { value: _ } => {
			// nothing uses an uint
		}
		MaterialParameter::Float { value } => {
			match parameter_name {
				"metallic" => mat.metallic = *value,
				"roughness" => mat.perceptual_roughness = *value,
				// we probably don't want to expose tex_scale
				// "tex_scale" => mat.tex_scale = *val,
				v => {
					error!("unknown param_name ({v}) for float")
				}
			}
		}
		MaterialParameter::Vec2 { value: _ } => {
			// nothing uses a Vec2
		}
		MaterialParameter::Vec3 { value: _ } => {
			// nothing uses a Vec3
		}
		MaterialParameter::Color { value: color } => match parameter_name {
			"color" => mat.base_color = color.to_bevy(),
			"emission_factor" => mat.emissive = color.to_bevy().to_linear(),
			v => {
				error!("unknown param_name ({v}) for color")
			}
		},
		MaterialParameter::Texture { value: resource } => {
			let Some(texture_path) = get_resource_file(
				resource,
				resource_prefixes,
				&[OsStr::new("png"), OsStr::new("jpg")],
			) else {
				return;
			};
			let handle = asset_server.load(texture_path);
			if let Ok(slot) = TextureSlot::from_str(parameter_name) {
				slot.get_part_texture(part_textures).replace((handle, None));
			} else {
				error!("unknown param_name ({parameter_name}) for texture");
			}
		}
		MaterialParameter::Dmatex {
			dmatex: _,
			acquire_point: _,
			release_point: _,
		} => {
			error!("somehow trying to handle a dmatex in the main material param path");
		}
	}
}

#[derive(Default, Debug)]
struct PartTextures {
	diffuse: Option<(Handle<Image>, Option<SignalOnDrop>)>,
	emission: Option<(Handle<Image>, Option<SignalOnDrop>)>,
	metal: Option<(Handle<Image>, Option<SignalOnDrop>)>,
	occlusion: Option<(Handle<Image>, Option<SignalOnDrop>)>,
}
#[derive(Hash, PartialEq, Eq, Clone, Copy, Debug)]
enum TextureSlot {
	Diffuse,
	Emission,
	Metal,
	Occlusion,
}
impl TextureSlot {
	fn get_part_texture<'a>(
		&self,
		textures: &'a mut PartTextures,
	) -> &'a mut Option<(Handle<Image>, Option<SignalOnDrop>)> {
		match self {
			TextureSlot::Diffuse => &mut textures.diffuse,
			TextureSlot::Emission => &mut textures.emission,
			TextureSlot::Metal => &mut textures.metal,
			TextureSlot::Occlusion => &mut textures.occlusion,
		}
	}
}
impl FromStr for TextureSlot {
	type Err = ();

	fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
		Ok(match s {
			"diffuse" => Self::Diffuse,
			"emission" => Self::Emission,
			"metal" => Self::Metal,
			"occlusion" => Self::Occlusion,
			_ => return Err(()),
		})
	}
}

#[derive(Debug)]
pub struct ModelPart {
	entity: OnceLock<EntityHandle>,
	mesh_entity: OnceLock<EntityHandle>,
	path: String,
	// TODO: replace spatial in model part with a custom system so we can switch spatials to purely
	// uniform scaling
	spatial: Arc<BinderObject<SpatialObject>>,
	pending_material_parameters: Mutex<FxHashMap<String, MaterialParameter>>,
	pending_material_replacement: Mutex<Option<Handle<BevyMaterial>>>,
	pending_dmatexes: Mutex<
		HashMap<
			TextureSlot,
			VecDeque<(
				oneshot::Receiver<(SignalOnDrop, Arc<BinderObject<Dmatex>>)>,
				AbortOnDrop,
			)>,
		>,
	>,
	textures: Mutex<PartTextures>,
	holdout: AtomicBool,
	bounds: OnceLock<Aabb>,
	bounding_calc: OnceLock<BoundingBoxCalc>,
	drop_notifs: RwLock<Vec<DropNotifier>>,
}
static ACQUIRE_SEMAPHORES: Mutex<Vec<Semaphore>> = Mutex::new(Vec::new());
impl ModelPart {
	pub fn replace_material(&self, replacement: Handle<BevyMaterial>) {
		self.pending_material_replacement
			.lock()
			.replace(replacement);
		*self.textures.lock() = PartTextures::default();
	}
	pub fn set_material_parameter(&self, parameter_name: String, value: MaterialParameter) {
		debug!(
			"setting material param: {parameter_name}: {value:?}, node_id: {:?}",
			self.mesh_entity.get(),
		);
		if let MaterialParameter::Dmatex {
			dmatex,
			acquire_point,
			release_point,
		} = value
		{
			let Ok(tex_slot) = TextureSlot::from_str(&parameter_name) else {
				error!("invalid texture slot: {parameter_name}");
				return;
			};
			let Some(tex) = dmatex.owned() else {
				error!("invalid dmatex");
				return;
			};
			let (tx, rx) = oneshot::channel();
			let tex = tex.clone();
			let task = tokio::spawn(async move {
				let release = tex.signal_on_drop(release_point);
				let Ok(future) = tex
					.timeline_sync()
					.wait_async(acquire_point)
					.inspect_err(|err| error!("unable to async wait on dmatex timeline: {err}"))
				else {
					return;
				};
				future.await;
				let sema = tex.get_acquire_semaphore(acquire_point);
				ACQUIRE_SEMAPHORES.lock().push(sema);
				tx.send((release, tex)).unwrap();
			});
			self.pending_dmatexes
				.lock()
				.entry(tex_slot)
				.or_default()
				.push_back((rx, task.into()));
		} else {
			self.pending_material_parameters
				.lock()
				.insert(parameter_name, value);
		}
	}
}

impl ModelPartHandler for ModelPart {
	async fn get_part_path(&self, _ctx: GluonCtx) -> String {
		self.path.clone()
	}

	async fn get_model_transform(&self, _ctx: GluonCtx) -> NonUniformTransform {
        // TODO: impl
		warn!("tried getting model part transform relative to model, currently unimplemented");
		NonUniformTransform {
			translation: Vec3::ZERO.into(),
			rotation: Quat::IDENTITY.into(),
			scale: Vec3::ONE.into(),
		}
	}

	async fn get_local_transform(&self, _ctx: GluonCtx) -> NonUniformTransform {
		let (scale, rotation, translation) = self
			.spatial
			.local_transform()
			.to_scale_rotation_translation();
		NonUniformTransform {
			translation: translation.into(),
			rotation: rotation.into(),
			scale: scale.into(),
		}
	}

	async fn get_relative_transform(
		&self,
		_ctx: GluonCtx,
		relative_to: ModelPartProxy,
	) -> NonUniformTransform {
        // TODO: make sure the 2 model parts are from the same model
		let Some(relative) = relative_to.owned() else {
			error!("unknown model part");
			return NonUniformTransform {
				translation: Vec3::ZERO.into(),
				rotation: Quat::IDENTITY.into(),
				scale: Vec3::ONE.into(),
			};
		};
		let (scale, rotation, translation) =
			Spatial::space_to_space_matrix(Some(&relative.spatial), Some(&self.spatial))
				.to_scale_rotation_translation();
		NonUniformTransform {
			translation: translation.into(),
			rotation: rotation.into(),
			scale: scale.into(),
		}
	}

	fn set_model_transform(&self, _ctx: GluonCtx, transform: PartialNonUniformTransform) {
        // TODO: impl
		warn!("tried setting model part transform relative to model, currently unimplemented");
	}

	fn set_local_transform(&self, _ctx: GluonCtx, transform: PartialNonUniformTransform) {
        // TODO: impl
		warn!("tried setting model part transform, currently unimplemented");
	}

	fn set_relative_transform(
		&self,
		_ctx: GluonCtx,
		_relative_to: ModelPartProxy,
		_transform: PartialNonUniformTransform,
	) {
        // TODO: impl
		warn!("tried setting model part transform relative to another model, currently unimplemented");
	}

	async fn set_material_parameter(
		&self,
		_ctx: GluonCtx,
		parameter_name: String,
		value: MaterialParameter,
	) -> Option<MaterialParamError> {
		if self.holdout.load(Ordering::Relaxed) {
			return Some(MaterialParamError::Holdout);
		}
		// TODO: return other errors
		self.set_material_parameter(parameter_name, value);
		None
	}

	fn apply_holdout_material(&self, _ctx: GluonCtx) {
		self.holdout.store(true, Ordering::Relaxed);
	}

	async fn drop_notification_requested(&self, notifier: DropNotifier) {
		self.drop_notifs.write().await.push(notifier);
	}
}
impl_proxy!(ModelPartProxy, ModelPart);
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

#[derive(Debug)]
pub struct Model {
	spatial: Arc<BinderObject<SpatialObject>>,
	_resource_id: Resource,
	bevy_scene_entity: OnceLock<EntityHandle>,
	parts: OnceLock<Vec<Arc<BinderObject<ModelPart>>>>,
	resource_prefixes: Arc<Vec<PathBuf>>,
	setup_complete_notify: Notify,
	drop_notifs: RwLock<Vec<DropNotifier>>,
}
impl Model {
	pub async fn new(
		spatial: Arc<BinderObject<SpatialObject>>,
		resource_id: Resource,
		base_prefixes: Arc<Vec<PathBuf>>,
	) -> Result<Arc<BinderObject<Model>>> {
		let pending_model_path = get_resource_file(
			&resource_id,
			base_prefixes.iter(),
			&[OsStr::new("glb"), OsStr::new("gltf")],
		)
		.ok_or_else(|| eyre!("Resource not found"))?;

		let model = PION.register_object(Model {
			spatial,
			_resource_id: resource_id,
			bevy_scene_entity: OnceLock::new(),
			parts: OnceLock::new(),
			resource_prefixes: base_prefixes,
			setup_complete_notify: Notify::new(),
			drop_notifs: RwLock::default(),
		});
		LOAD_MODEL
			.send((model.clone(), pending_model_path))
			.unwrap();
		MODEL_REGISTRY.add_raw(&model);
		ref_owned(&model);
		model.setup_complete_notify.notified().await;

		Ok(model)
	}
}
impl ModelHandler for Model {
	async fn get_spatial(&self, _ctx: GluonCtx) -> SpatialProxy {
        SpatialProxy::from_handler(&self.spatial)
	}

	async fn get_part(&self, _ctx: GluonCtx, path: String) -> Option<ModelPartProxy> {
		if let Some(parts) = self.parts.get() {
			parts
				.iter()
				.find(|p| p.path == path)
				.map(ModelPartProxy::from_handler)
		} else {
			error!(
				"somehow called get_part before model parts were initialized, should be unreachable"
			);
			None
		}
	}

	async fn enumerate_parts(&self, _ctx: GluonCtx) -> Vec<ModelPartProxy> {
		if let Some(parts) = self.parts.get() {
			parts.iter().map(ModelPartProxy::from_handler).collect()
		} else {
			error!(
				"somehow called enumerate_parts before model parts were initialized, should be unreachable"
			);
			Vec::new()
		}
	}

	fn set_model_scale(&self, _ctx: GluonCtx, scale: Vec3F) {
        // TODO: impl
		warn!("tried setting model scale, currently unimplemented");
	}

	async fn drop_notification_requested(&self, notifier: DropNotifier) {
		self.drop_notifs.write().await.push(notifier);
	}
}
interface!(ModelInterface);
impl ModelInterfaceHandler for ModelInterface {
	async fn load_model(
		&self,
		_ctx: gluon_wire::GluonCtx,
		spatial: stardust_xr_protocol::spatial::Spatial,
		model: stardust_xr_protocol::types::Resource,
		model_scale: stardust_xr_protocol::types::Vec3F,
	) -> ModelProxy {
		let Some(spatial) = spatial.owned() else {
			// TODO: replace with proper error returning
			panic!("invalid spatial in model loading");
		};

		// TODO: handle
		let model = Model::new(spatial, model, self.base_resource_prefixes.clone())
			.await
			.unwrap();
		ModelProxy::from_handler(&model)
	}

	async fn drop_notification_requested(&self, notifier: DropNotifier) {
		self.drop_notifs.write().await.push(notifier);
	}
}
impl_transaction_handler!(Model);
impl_transaction_handler!(ModelPart);

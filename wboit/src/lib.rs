mod render;

use bevy::{
	asset::{load_internal_asset, weak_handle},
	ecs::{component::HookContext, world::DeferredWorld},
	pbr::{ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline},
	platform::collections::HashMap,
	prelude::*,
	render::{
		RenderApp,
		extract_component::{ExtractComponent, ExtractComponentPlugin},
		mesh::MeshVertexBufferLayoutRef,
		render_resource::{
			AsBindGroup, RenderPipelineDescriptor, ShaderDefVal, ShaderRef,
			SpecializedMeshPipelineError,
		},
	},
};

const WBOIT_SHADER_HANDLE: Handle<Shader> = weak_handle!("3e0f7a5c-6a2b-4f39-9d7e-1c5b8e2f4a61");
const PBR_SHADER_HANDLE: Handle<Shader> = weak_handle!("a9c14d27-5b8e-4e0a-b3f6-7d2e9c1a0b58");
const RESOLVE_SHADER_HANDLE: Handle<Shader> = weak_handle!("5d7b2e91-0c4f-4a86-9e13-b8f6a2d4c730");
const COMPOSITE_SHADER_HANDLE: Handle<Shader> =
	weak_handle!("c2e86f14-9a3d-4b75-8f01-6e4a3b9d2c87");

pub type WboitMaterial = ExtendedMaterial<StandardMaterial, WboitExtension>;

/// histogram equalized weighted blended order independent transparency
///
/// the depth histogram from last frame remaps each fragment's depth to its quantile of
/// the tile's optical depth, so the weights spread evenly across the fragments actually there
#[derive(Component, ExtractComponent, Clone, Copy, PartialEq, Debug, Reflect)]
#[reflect(Component, Default)]
pub struct Wboit {
	/// the biggest quality knob, smaller tiles stop pixels with different depth profiles
	/// from sharing a histogram
	pub tile_size: u32,

	/// depths are binned on a log scale between these, in meters
	pub near: f32,
	pub far: f32,
	/// two layers closer together than a bin can't be told apart
	pub bins: Bins,
}
impl Default for Wboit {
	fn default() -> Self {
		Self {
			tile_size: 8,
			near: 0.05,
			far: 50.0,
			bins: Bins::B16,
		}
	}
}

/// four bins pack into each rgba16f render target of the histogram pass,
/// so the discriminant is how many targets that takes
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Reflect)]
pub enum Bins {
	B4 = 1,
	B8 = 2,
	B12 = 3,
	#[default]
	B16 = 4,
	B20 = 5,
	B24 = 6,
	B28 = 7,
	B32 = 8,
}
impl Bins {
	pub fn layers(self) -> u32 {
		self as u32
	}
}

/// a material's transparent pipelines are only drawn with wboit when marked,
/// its fragment shader must return `stardust_wboit::wboit_output`
pub fn enable(descriptor: &mut RenderPipelineDescriptor) {
	if let Some(fragment) = descriptor.fragment.as_mut() {
		fragment.shader_defs.push("WBOIT".into());
		// naga_oil evaluates `#if WBOIT_LAYERS` even in inactive branches, the passes override it
		fragment
			.shader_defs
			.push(ShaderDefVal::UInt("WBOIT_LAYERS".into(), 1));
	}
}

#[derive(Default, Asset, AsBindGroup, TypePath, Debug, Clone)]
#[data(50, u32, binding_array(101))]
#[bindless(index_table(range(50..51), binding(100)))]
pub struct WboitExtension {}
impl From<&WboitExtension> for u32 {
	fn from(_: &WboitExtension) -> Self {
		0
	}
}
impl MaterialExtension for WboitExtension {
	fn fragment_shader() -> ShaderRef {
		PBR_SHADER_HANDLE.into()
	}

	fn specialize(
		_pipeline: &MaterialExtensionPipeline,
		descriptor: &mut RenderPipelineDescriptor,
		_layout: &MeshVertexBufferLayoutRef,
		_key: MaterialExtensionKey<Self>,
	) -> Result<(), SpecializedMeshPipelineError> {
		enable(descriptor);
		Ok(())
	}
}

pub struct WboitPlugin;
impl Plugin for WboitPlugin {
	fn build(&self, app: &mut App) {
		load_internal_asset!(app, WBOIT_SHADER_HANDLE, "wboit.wgsl", Shader::from_wgsl);
		load_internal_asset!(app, PBR_SHADER_HANDLE, "pbr.wgsl", Shader::from_wgsl);
		load_internal_asset!(
			app,
			RESOLVE_SHADER_HANDLE,
			"resolve.wgsl",
			Shader::from_wgsl
		);
		load_internal_asset!(
			app,
			COMPOSITE_SHADER_HANDLE,
			"composite.wgsl",
			Shader::from_wgsl
		);

		app.register_type::<Wboit>().add_plugins((
			ExtractComponentPlugin::<Wboit>::default(),
			MaterialPlugin::<WboitMaterial>::default(),
		));

		if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
			render::build(render_app);
		}
	}

	fn finish(&self, app: &mut App) {
		if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
			render::finish(render_app);
		}
	}
}

/// swaps every inserted `MeshMaterial3d<StandardMaterial>` for a `WboitMaterial` copy,
/// the copy is a snapshot so later edits to the standard material don't carry over
pub struct SwapStandardMaterialPlugin;
impl Plugin for SwapStandardMaterialPlugin {
	fn build(&self, app: &mut App) {
		app.init_resource::<Mirrors>();
		app.world_mut()
			.register_component_hooks::<MeshMaterial3d<StandardMaterial>>()
			.on_insert(swap);
	}
}

#[derive(Resource, Default)]
struct Mirrors(HashMap<AssetId<StandardMaterial>, Handle<WboitMaterial>>);

fn swap(mut world: DeferredWorld, ctx: HookContext) {
	let Some(id) = world
		.get::<MeshMaterial3d<StandardMaterial>>(ctx.entity)
		.map(|m| m.id())
	else {
		return;
	};
	let mirrored = world.resource::<Mirrors>().0.get(&id).map(|h| h.id());
	let mirrored = mirrored.and_then(|h| {
		world
			.resource_mut::<Assets<WboitMaterial>>()
			.get_strong_handle(h)
	});
	let handle = match mirrored {
		Some(h) => h,
		None => {
			let Some(base) = world
				.resource::<Assets<StandardMaterial>>()
				.get(id)
				.cloned()
			else {
				return;
			};
			let h = world
				.resource_mut::<Assets<WboitMaterial>>()
				.add(WboitMaterial {
					base,
					extension: WboitExtension {},
				});
			world.resource_mut::<Mirrors>().0.insert(id, h.clone_weak());
			h
		}
	};
	world
		.commands()
		.entity(ctx.entity)
		.remove::<MeshMaterial3d<StandardMaterial>>()
		.insert(MeshMaterial3d(handle));
}

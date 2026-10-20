use crate::Wboit;
use bevy::{
	core_pipeline::core_3d::{Opaque3dBatchSetKey, Opaque3dBinKey, Transparent3d},
	pbr::{DrawMesh, Material, SetMaterialBindGroup, SetMeshBindGroup, SetMeshViewBindGroup},
	platform::collections::{HashMap, HashSet},
	prelude::*,
	render::{
		Extract, RenderApp,
		batching::gpu_preprocessing::{GpuPreprocessingMode, GpuPreprocessingSupport},
		render_phase::{
			AddRenderCommand, BinnedPhaseItem, CachedRenderPipelinePhaseItem, DrawFunctionId,
			DrawFunctions, PhaseItem, PhaseItemExtraIndex, SetItemPipeline, ViewBinnedRenderPhases,
		},
		render_resource::CachedRenderPipelineId,
		sync_world::MainEntity,
		view::{NoIndirectDrawing, RetainedViewEntity},
	},
};
use std::{hash::Hash, marker::PhantomData, ops::Range};

pub struct Prepass;
pub struct Accum;

/// wboit doesn't care about draw order, so its items get binned like opaque ones and
/// runs of different meshes can go out as a single multi draw
pub struct WboitItem<P> {
	batch_set_key: Opaque3dBatchSetKey,
	_bin_key: Opaque3dBinKey,
	representative_entity: (Entity, MainEntity),
	batch_range: Range<u32>,
	extra_index: PhaseItemExtraIndex,
	_pass: PhantomData<fn() -> P>,
}
impl<P: 'static> PhaseItem for WboitItem<P> {
	fn entity(&self) -> Entity {
		self.representative_entity.0
	}

	fn main_entity(&self) -> MainEntity {
		self.representative_entity.1
	}

	fn draw_function(&self) -> DrawFunctionId {
		self.batch_set_key.draw_function
	}

	fn batch_range(&self) -> &Range<u32> {
		&self.batch_range
	}

	fn batch_range_mut(&mut self) -> &mut Range<u32> {
		&mut self.batch_range
	}

	fn extra_index(&self) -> PhaseItemExtraIndex {
		self.extra_index.clone()
	}

	fn batch_range_and_extra_index_mut(&mut self) -> (&mut Range<u32>, &mut PhaseItemExtraIndex) {
		(&mut self.batch_range, &mut self.extra_index)
	}
}
impl<P: 'static> BinnedPhaseItem for WboitItem<P> {
	type BatchSetKey = Opaque3dBatchSetKey;
	type BinKey = Opaque3dBinKey;

	fn new(
		batch_set_key: Self::BatchSetKey,
		bin_key: Self::BinKey,
		representative_entity: (Entity, MainEntity),
		batch_range: Range<u32>,
		extra_index: PhaseItemExtraIndex,
	) -> Self {
		Self {
			batch_set_key,
			_bin_key: bin_key,
			representative_entity,
			batch_range,
			extra_index,
			_pass: PhantomData,
		}
	}
}
impl<P: 'static> CachedRenderPipelinePhaseItem for WboitItem<P> {
	fn cached_pipeline(&self) -> CachedRenderPipelineId {
		self.batch_set_key.pipeline
	}
}

// identical to bevy's private DrawMaterial, so it shares its TypeId in DrawFunctions<Transparent3d>
type DrawWboit<M> = (
	SetItemPipeline,
	SetMeshViewBindGroup<0>,
	SetMeshBindGroup<1>,
	SetMaterialBindGroup<M, 2>,
	DrawMesh,
);

/// a material's transparent draw function -> its draw functions in the prepass and accum
#[derive(Resource, Default, Deref)]
pub(crate) struct WboitDraws(HashMap<DrawFunctionId, [DrawFunctionId; 2]>);

/// lets a material marked with [`crate::enable`] draw through the wboit passes,
/// [`crate::WboitPlugin`] already adds it for [`crate::WboitMaterial`]
pub struct WboitMaterialPlugin<M>(PhantomData<M>);
impl<M> Default for WboitMaterialPlugin<M> {
	fn default() -> Self {
		Self(PhantomData)
	}
}
impl<M: Material> Plugin for WboitMaterialPlugin<M>
where
	M::Data: PartialEq + Eq + Hash + Clone,
{
	fn build(&self, app: &mut App) {
		if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
			render_app
				.init_resource::<WboitDraws>()
				.init_resource::<DrawFunctions<WboitItem<Prepass>>>()
				.init_resource::<DrawFunctions<WboitItem<Accum>>>()
				.add_render_command::<WboitItem<Prepass>, DrawWboit<M>>()
				.add_render_command::<WboitItem<Accum>, DrawWboit<M>>();
		}
	}

	fn finish(&self, app: &mut App) {
		let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
			return;
		};
		let world = render_app.world_mut();
		let Some(transparent) = world
			.resource::<DrawFunctions<Transparent3d>>()
			.read()
			.get_id::<DrawWboit<M>>()
		else {
			error!(
				"{} has no transparent draw function, add its MaterialPlugin first",
				std::any::type_name::<M>()
			);
			return;
		};
		let prepass = world
			.resource::<DrawFunctions<WboitItem<Prepass>>>()
			.read()
			.id::<DrawWboit<M>>();
		let accum = world
			.resource::<DrawFunctions<WboitItem<Accum>>>()
			.read()
			.id::<DrawWboit<M>>();
		world
			.resource_mut::<WboitDraws>()
			.0
			.insert(transparent, [prepass, accum]);
	}
}

pub(crate) fn extract_phases(
	mut prepass: ResMut<ViewBinnedRenderPhases<WboitItem<Prepass>>>,
	mut accum: ResMut<ViewBinnedRenderPhases<WboitItem<Accum>>>,
	cameras: Extract<
		Query<(Entity, &Camera, Has<NoIndirectDrawing>), (With<Camera3d>, With<Wboit>)>,
	>,
	support: Res<GpuPreprocessingSupport>,
	mut live: Local<HashSet<RetainedViewEntity>>,
) {
	live.clear();
	for (entity, camera, no_indirect) in &cameras {
		if !camera.is_active {
			continue;
		}
		let mode = support.min(match no_indirect {
			true => GpuPreprocessingMode::PreprocessingOnly,
			false => GpuPreprocessingMode::Culling,
		});
		let view = RetainedViewEntity::new(entity.into(), None, 0);
		prepass.prepare_for_new_frame(view, mode);
		accum.prepare_for_new_frame(view, mode);
		live.insert(view);
	}
	prepass.retain(|v, _| live.contains(v));
	accum.retain(|v, _| live.contains(v));
}

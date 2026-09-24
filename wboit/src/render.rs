use crate::{COMPOSITE_SHADER_HANDLE, RESOLVE_SHADER_HANDLE, Wboit};
use bevy::{
	core_pipeline::{
		core_3d::{
			Transparent3d,
			graph::{Core3d, Node3d},
		},
		fullscreen_vertex_shader::fullscreen_shader_vertex_state,
	},
	ecs::query::QueryItem,
	platform::collections::{HashMap, HashSet},
	prelude::*,
	render::{
		Render, RenderSet,
		camera::ExtractedCamera,
		render_graph::{
			NodeRunError, RenderGraphApp, RenderGraphContext, RenderLabel, ViewNode, ViewNodeRunner,
		},
		render_phase::{SortedRenderPhase, ViewSortedRenderPhases},
		render_resource::{
			binding_types::{sampler, texture_2d, texture_2d_array, uniform_buffer},
			*,
		},
		renderer::{RenderContext, RenderDevice, RenderQueue},
		view::{ExtractedView, RetainedViewEntity, ViewDepthTexture, ViewTarget},
	},
};
use std::mem;

const HISTOGRAM: TextureFormat = TextureFormat::Rgba16Float;

pub fn build(render_app: &mut SubApp) {
	render_app
		.init_resource::<WboitViews>()
		.add_systems(
			Render,
			(
				prepare_views.in_set(RenderSet::PrepareResources),
				split_phases.in_set(RenderSet::PrepareBindGroups),
			),
		)
		.add_render_graph_node::<ViewNodeRunner<WboitNode>>(Core3d, WboitLabel)
		.add_render_graph_edges(
			Core3d,
			(
				Node3d::MainTransmissivePass,
				WboitLabel,
				Node3d::MainTransparentPass,
			),
		);
}

pub fn finish(render_app: &mut SubApp) {
	render_app.init_resource::<WboitPipelines>();
}

// encase's derive emits module level check fns that never get called
#[allow(dead_code)]
mod params {
	use bevy::{math::UVec2, render::render_resource::ShaderType};

	#[derive(ShaderType, Clone, Copy)]
	pub struct WboitParams {
		pub tiles: UVec2,
		pub tile_size: u32,
		pub near: f32,
		pub depth_scale: f32,
	}
}
use params::WboitParams;

#[derive(Resource)]
struct WboitPipelines {
	draw_layout: BindGroupLayout,
	resolve_layout: BindGroupLayout,
	composite_layout: BindGroupLayout,
	sampler: Sampler,
	// keyed by histogram layers
	resolve: HashMap<u32, CachedRenderPipelineId>,
	composite: HashMap<TextureFormat, CachedRenderPipelineId>,
	// (material pipeline, layers) -> [accum, bin], None when it isn't marked for wboit
	derived: HashMap<(CachedRenderPipelineId, u32), Option<[CachedRenderPipelineId; 2]>>,
}
impl FromWorld for WboitPipelines {
	fn from_world(world: &mut World) -> Self {
		let device = world.resource::<RenderDevice>();
		let tex = || texture_2d(TextureSampleType::Float { filterable: true });
		let array = || texture_2d_array(TextureSampleType::Float { filterable: true });
		let draw_layout = device.create_bind_group_layout(
			"wboit_draw",
			&BindGroupLayoutEntries::sequential(
				ShaderStages::FRAGMENT,
				(
					uniform_buffer::<WboitParams>(false),
					array(),
					sampler(SamplerBindingType::Filtering),
					tex(),
					tex(),
				),
			),
		);
		let resolve_layout = device.create_bind_group_layout(
			"wboit_resolve",
			&BindGroupLayoutEntries::single(ShaderStages::FRAGMENT, array()),
		);
		let composite_layout = device.create_bind_group_layout(
			"wboit_composite",
			&BindGroupLayoutEntries::sequential(ShaderStages::FRAGMENT, (tex(), tex())),
		);
		let sampler = device.create_sampler(&SamplerDescriptor {
			label: Some("wboit_cdf"),
			mag_filter: FilterMode::Linear,
			min_filter: FilterMode::Linear,
			..default()
		});
		Self {
			draw_layout,
			resolve_layout,
			composite_layout,
			sampler,
			resolve: default(),
			composite: default(),
			derived: default(),
		}
	}
}
impl WboitPipelines {
	fn resolve(&mut self, cache: &PipelineCache, layers: u32) {
		let layout = self.resolve_layout.clone();
		self.resolve.entry(layers).or_insert_with(|| {
			cache.queue_render_pipeline(RenderPipelineDescriptor {
				label: Some("wboit_resolve".into()),
				layout: vec![layout],
				push_constant_ranges: vec![],
				vertex: fullscreen_shader_vertex_state(),
				fragment: Some(FragmentState {
					shader: RESOLVE_SHADER_HANDLE,
					shader_defs: vec![ShaderDefVal::UInt("WBOIT_LAYERS".into(), layers)],
					entry_point: "fragment".into(),
					targets: vec![Some(HISTOGRAM.into()); layers as usize],
				}),
				primitive: default(),
				depth_stencil: None,
				multisample: default(),
				zero_initialize_workgroup_memory: false,
			})
		});
	}

	fn composite(&mut self, cache: &PipelineCache, format: TextureFormat) {
		let layout = self.composite_layout.clone();
		self.composite.entry(format).or_insert_with(|| {
			cache.queue_render_pipeline(RenderPipelineDescriptor {
				label: Some("wboit_composite".into()),
				layout: vec![layout],
				push_constant_ranges: vec![],
				vertex: fullscreen_shader_vertex_state(),
				fragment: Some(FragmentState {
					shader: COMPOSITE_SHADER_HANDLE,
					shader_defs: vec![],
					entry_point: "fragment".into(),
					targets: vec![Some(ColorTargetState {
						format,
						blend: Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING),
						write_mask: ColorWrites::ALL,
					})],
				}),
				primitive: default(),
				depth_stencil: None,
				multisample: default(),
				zero_initialize_workgroup_memory: false,
			})
		});
	}

	/// outer None when the material pipeline hasn't reached the cache yet
	fn derive(
		&self,
		cache: &PipelineCache,
		id: CachedRenderPipelineId,
		layers: u32,
	) -> Option<Option<[CachedRenderPipelineId; 2]>> {
		let PipelineDescriptor::RenderPipelineDescriptor(d) =
			&cache.pipelines().nth(id.id())?.descriptor
		else {
			return Some(None);
		};
		if !d
			.fragment
			.as_ref()
			.is_some_and(|f| f.shader_defs.contains(&"WBOIT".into()))
		{
			return Some(None);
		}
		let additive = |format: TextureFormat| {
			Some(ColorTargetState {
				format,
				blend: Some(BlendState {
					color: BlendComponent {
						src_factor: BlendFactor::One,
						dst_factor: BlendFactor::One,
						operation: BlendOperation::Add,
					},
					alpha: BlendComponent {
						src_factor: BlendFactor::One,
						dst_factor: BlendFactor::One,
						operation: BlendOperation::Add,
					},
				}),
				write_mask: ColorWrites::ALL,
			})
		};
		let variant = |pass: &'static str, targets: Vec<Option<ColorTargetState>>| {
			let mut d = (**d).clone();
			d.label = Some(pass.to_lowercase().into());
			d.layout.push(self.draw_layout.clone());
			let f = d.fragment.as_mut().unwrap();
			f.shader_defs
				.retain(|d| !matches!(d, ShaderDefVal::UInt(n, _) if n == "WBOIT_LAYERS"));
			f.shader_defs.extend([
				"WBOIT_PASS".into(),
				pass.into(),
				ShaderDefVal::UInt("WBOIT_LAYERS".into(), layers),
			]);
			f.targets = targets;
			d
		};
		let min = BlendComponent {
			src_factor: BlendFactor::One,
			dst_factor: BlendFactor::One,
			operation: BlendOperation::Min,
		};
		let accum = variant(
			"WBOIT_ACCUM",
			vec![
				additive(TextureFormat::Rgba16Float),
				additive(TextureFormat::R16Float),
				Some(ColorTargetState {
					format: TextureFormat::R16Float,
					blend: Some(BlendState {
						color: min,
						alpha: min,
					}),
					write_mask: ColorWrites::ALL,
				}),
			],
		);
		// rasterized at one pixel per tile, a depth buffer that small can't mean anything
		let mut bin = variant("WBOIT_BIN", vec![additive(HISTOGRAM); layers as usize]);
		bin.depth_stencil = None;
		Some(Some([
			cache.queue_render_pipeline(accum),
			cache.queue_render_pipeline(bin),
		]))
	}
}

#[derive(Resource, Default, Deref, DerefMut)]
struct WboitViews(HashMap<RetainedViewEntity, ViewState>);

struct ViewState {
	wboit: Wboit,
	size: UVec2,
	tiles: UVec2,
	// written this frame, the other half of each pair is last frame's
	frame: usize,

	accum: TextureView,
	tau: [TextureView; 2],
	front: [TextureView; 2],
	// one attachment view per layer, the arrays themselves live in the bind groups
	hist: Vec<TextureView>,
	cdf: [Vec<TextureView>; 2],

	draw: [BindGroup; 2],
	composite: [BindGroup; 2],
	resolve: BindGroup,

	accum_phase: SortedRenderPhase<Transparent3d>,
	bin_phase: SortedRenderPhase<Transparent3d>,
}
impl ViewState {
	fn new(
		device: &RenderDevice,
		queue: &RenderQueue,
		pipelines: &WboitPipelines,
		wboit: Wboit,
		size: UVec2,
	) -> Self {
		let tiles = (size + UVec2::splat(wboit.tile_size - 1)) / wboit.tile_size;
		let layers = wboit.bins.layers();
		let texture = |label: &'static str, size: UVec2, format: TextureFormat, layers: u32| {
			device.create_texture(&TextureDescriptor {
				label: Some(label),
				size: Extent3d {
					width: size.x,
					height: size.y,
					depth_or_array_layers: layers,
				},
				mip_level_count: 1,
				sample_count: 1,
				dimension: TextureDimension::D2,
				format,
				usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
				view_formats: &[],
			})
		};
		let tex = |label, format| texture(label, size, format, 1).create_view(&default());
		let histogram = |label| {
			let t = texture(label, tiles, HISTOGRAM, layers);
			let array = t.create_view(&TextureViewDescriptor {
				dimension: Some(TextureViewDimension::D2Array),
				..default()
			});
			let views = (0..layers)
				.map(|l| {
					t.create_view(&TextureViewDescriptor {
						dimension: Some(TextureViewDimension::D2),
						base_array_layer: l,
						array_layer_count: Some(1),
						..default()
					})
				})
				.collect::<Vec<_>>();
			(array, views)
		};
		let accum = tex("wboit_accum", TextureFormat::Rgba16Float);
		let tau = [(); 2].map(|_| tex("wboit_tau", TextureFormat::R16Float));
		let front = [(); 2].map(|_| tex("wboit_front", TextureFormat::R16Float));
		let (hist_array, hist) = histogram("wboit_hist");
		let cdf = [(); 2].map(|_| histogram("wboit_cdf"));

		let mut params = UniformBuffer::from(WboitParams {
			tiles,
			tile_size: wboit.tile_size,
			near: wboit.near,
			depth_scale: 1.0 / (wboit.far / wboit.near).ln(),
		});
		params.write_buffer(device, queue);

		let draw = [0, 1].map(|prev: usize| {
			device.create_bind_group(
				"wboit_draw",
				&pipelines.draw_layout,
				&BindGroupEntries::sequential((
					params.binding().unwrap(),
					&cdf[prev].0,
					&pipelines.sampler,
					&tau[prev],
					&front[prev],
				)),
			)
		});
		let composite = [0, 1].map(|frame: usize| {
			device.create_bind_group(
				"wboit_composite",
				&pipelines.composite_layout,
				&BindGroupEntries::sequential((&accum, &tau[frame])),
			)
		});
		let resolve = device.create_bind_group(
			"wboit_resolve",
			&pipelines.resolve_layout,
			&BindGroupEntries::single(&hist_array),
		);

		Self {
			wboit,
			size,
			tiles,
			frame: 0,
			accum,
			tau,
			front,
			hist,
			cdf: cdf.map(|(_, layers)| layers),
			draw,
			composite,
			resolve,
			accum_phase: default(),
			bin_phase: default(),
		}
	}
}

fn prepare_views(
	mut views: ResMut<WboitViews>,
	mut pipelines: ResMut<WboitPipelines>,
	cameras: Query<(&ExtractedView, &ExtractedCamera, &ViewTarget, &Wboit)>,
	device: Res<RenderDevice>,
	queue: Res<RenderQueue>,
	cache: Res<PipelineCache>,
	mut live: Local<HashSet<RetainedViewEntity>>,
) {
	live.clear();
	let limits = device.limits();
	let max_layers = limits.max_color_attachments.min(
		limits.max_color_attachment_bytes_per_sample / HISTOGRAM.block_copy_size(None).unwrap(),
	);
	for (view, camera, target, wboit) in &cameras {
		let Some(size) = camera.physical_target_size else {
			continue;
		};
		if wboit.bins.layers() > max_layers {
			error_once!(
				"{:?} needs {} render targets but this device only allows {max_layers}, falling back to alpha blending",
				wboit.bins,
				wboit.bins.layers(),
			);
			continue;
		}
		live.insert(view.retained_view_entity);
		pipelines.composite(&cache, target.main_texture_format());
		pipelines.resolve(&cache, wboit.bins.layers());
		match views.get_mut(&view.retained_view_entity) {
			Some(state) if state.size == size && state.wboit == *wboit => state.frame ^= 1,
			_ => {
				let state = ViewState::new(&device, &queue, &pipelines, *wboit, size);
				views.insert(view.retained_view_entity, state);
			}
		}
	}
	views.retain(|v, _| live.contains(v));
}

// runs after batching so every batch lands whole on one side, it can only span items
// that share a pipeline
fn split_phases(
	mut phases: ResMut<ViewSortedRenderPhases<Transparent3d>>,
	mut views: ResMut<WboitViews>,
	mut pipelines: ResMut<WboitPipelines>,
	cache: Res<PipelineCache>,
) {
	for (view, state) in views.iter_mut() {
		state.accum_phase.items.clear();
		state.bin_phase.items.clear();
		let Some(phase) = phases.get_mut(view) else {
			continue;
		};
		let layers = state.wboit.bins.layers();
		for item in mem::take(&mut phase.items) {
			let key = (item.pipeline, layers);
			let derived = match pipelines.derived.get(&key) {
				Some(d) => *d,
				None => match pipelines.derive(&cache, item.pipeline, layers) {
					Some(d) => *pipelines.derived.entry(key).or_insert(d),
					None => None,
				},
			};
			let Some([accum, bin]) = derived else {
				phase.items.push(item);
				continue;
			};
			state.bin_phase.add(Transparent3d {
				pipeline: bin,
				batch_range: item.batch_range.clone(),
				extra_index: item.extra_index.clone(),
				..item
			});
			state.accum_phase.add(Transparent3d {
				pipeline: accum,
				..item
			});
		}
	}
}

#[derive(RenderLabel, Debug, Clone, Hash, PartialEq, Eq)]
struct WboitLabel;

#[derive(Default)]
struct WboitNode;
impl ViewNode for WboitNode {
	type ViewQuery = (
		&'static ExtractedCamera,
		&'static ExtractedView,
		&'static ViewTarget,
		&'static ViewDepthTexture,
	);

	fn run<'w>(
		&self,
		graph: &mut RenderGraphContext,
		render_context: &mut RenderContext<'w>,
		(camera, view, target, depth): QueryItem<'w, Self::ViewQuery>,
		world: &'w World,
	) -> Result<(), NodeRunError> {
		let Some(state) = world
			.resource::<WboitViews>()
			.get(&view.retained_view_entity)
		else {
			return Ok(());
		};
		if state.accum_phase.items.is_empty() {
			return Ok(());
		}
		let pipelines = world.resource::<WboitPipelines>();
		let cache = world.resource::<PipelineCache>();
		let view_entity = graph.view_entity();
		let f = state.frame;

		{
			let mut pass = render_context.begin_tracked_render_pass(RenderPassDescriptor {
				label: Some("wboit_accum"),
				color_attachments: &[
					clear(&state.accum),
					clear(&state.tau[f]),
					Some(RenderPassColorAttachment {
						view: &state.front[f],
						resolve_target: None,
						ops: Operations {
							load: LoadOp::Clear(LinearRgba::WHITE.into()),
							store: StoreOp::Store,
						},
					}),
				],
				depth_stencil_attachment: Some(depth.get_attachment(StoreOp::Store)),
				timestamp_writes: None,
				occlusion_query_set: None,
			});
			if let Some(viewport) = camera.viewport.as_ref() {
				pass.set_camera_viewport(viewport);
			}
			pass.set_bind_group(3, &state.draw[f ^ 1], &[]);
			if let Err(err) = state.accum_phase.render(&mut pass, world, view_entity) {
				error!("error rendering the wboit accum phase: {err:?}");
			}
		}

		if let Some(composite) = pipelines
			.composite
			.get(&target.main_texture_format())
			.and_then(|id| cache.get_render_pipeline(*id))
		{
			let mut pass = render_context.begin_tracked_render_pass(RenderPassDescriptor {
				label: Some("wboit_composite"),
				color_attachments: &[Some(target.get_color_attachment())],
				depth_stencil_attachment: None,
				timestamp_writes: None,
				occlusion_query_set: None,
			});
			if let Some(viewport) = camera.viewport.as_ref() {
				pass.set_camera_viewport(viewport);
			}
			pass.set_render_pipeline(composite);
			pass.set_bind_group(0, &state.composite[f], &[]);
			pass.draw(0..3, 0..1);
		}

		{
			let mut pass = render_context.begin_tracked_render_pass(RenderPassDescriptor {
				label: Some("wboit_bin"),
				color_attachments: &state.hist.iter().map(clear).collect::<Vec<_>>(),
				depth_stencil_attachment: None,
				timestamp_writes: None,
				occlusion_query_set: None,
			});
			// squeezes full res pixel p onto tile p / tile_size, the target is rounded up to
			// whole tiles so the edge stays aligned with the cdf lookup
			let (pos, size) = camera
				.viewport
				.as_ref()
				.map(|v| (v.physical_position, v.physical_size))
				.unwrap_or((UVec2::ZERO, state.size));
			let s = 1.0 / state.wboit.tile_size as f32;
			let pos = pos.as_vec2() * s;
			let size = (size.as_vec2() * s).min(state.tiles.as_vec2() - pos);
			pass.set_viewport(pos.x, pos.y, size.x, size.y, 0.0, 1.0);
			pass.set_bind_group(3, &state.draw[f ^ 1], &[]);
			if let Err(err) = state.bin_phase.render(&mut pass, world, view_entity) {
				error!("error rendering the wboit bin phase: {err:?}");
			}
		}

		if let Some(resolve) = pipelines
			.resolve
			.get(&state.wboit.bins.layers())
			.and_then(|id| cache.get_render_pipeline(*id))
		{
			let mut pass = render_context.begin_tracked_render_pass(RenderPassDescriptor {
				label: Some("wboit_resolve"),
				color_attachments: &state.cdf[f].iter().map(clear).collect::<Vec<_>>(),
				depth_stencil_attachment: None,
				timestamp_writes: None,
				occlusion_query_set: None,
			});
			pass.set_render_pipeline(resolve);
			pass.set_bind_group(0, &state.resolve, &[]);
			pass.draw(0..3, 0..1);
		}

		Ok(())
	}
}

fn clear(view: &TextureView) -> Option<RenderPassColorAttachment<'_>> {
	Some(RenderPassColorAttachment {
		view,
		resolve_target: None,
		ops: Operations {
			load: LoadOp::Clear(default()),
			store: StoreOp::Store,
		},
	})
}

use crate::{COMPOSITE_SHADER_HANDLE, CdfScope, HISTOGRAM_SHADER_HANDLE, Wboit};
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
			binding_types::{
				sampler, storage_buffer_read_only_sized, storage_buffer_sized, texture_2d,
				texture_2d_array, texture_depth_2d, texture_storage_2d_array, uniform_buffer,
			},
			*,
		},
		renderer::{RenderContext, RenderDevice, RenderQueue},
		view::{ExtractedView, RetainedViewEntity, ViewDepthTexture, ViewTarget},
	},
};
use std::mem;

const CDF: TextureFormat = TextureFormat::Rgba16Float;
const PIXEL: TextureFormat = TextureFormat::Rgba16Float;

#[derive(Clone)]
struct PixelHistogram {
	array: TextureView,
	// one attachment per layer of four bins
	layers: Vec<TextureView>,
}
impl PixelHistogram {
	fn new(device: &RenderDevice, size: UVec2, layers: u32) -> Self {
		let t = device.create_texture(&TextureDescriptor {
			label: Some("wboit_pixel_hist"),
			size: Extent3d {
				width: size.x,
				height: size.y,
				depth_or_array_layers: layers,
			},
			mip_level_count: 1,
			sample_count: 1,
			dimension: TextureDimension::D2,
			format: PIXEL,
			usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
			view_formats: &[],
		});
		Self {
			array: t.create_view(&TextureViewDescriptor {
				dimension: Some(TextureViewDimension::D2Array),
				..default()
			}),
			layers: (0..layers)
				.map(|l| {
					t.create_view(&TextureViewDescriptor {
						dimension: Some(TextureViewDimension::D2),
						base_array_layer: l,
						array_layer_count: Some(1),
						..default()
					})
				})
				.collect(),
		}
	}
}

pub fn build(render_app: &mut SubApp) {
	render_app
		.init_resource::<WboitViews>()
		.add_systems(
			Render,
			(
				prepare_views.in_set(RenderSet::PrepareResources),
				(split_phases, bind_prepass).in_set(RenderSet::PrepareBindGroups),
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
	prepass_layout: BindGroupLayout,
	accum_layout: BindGroupLayout,
	pixel_layout: BindGroupLayout,
	histogram_layout: BindGroupLayout,
	composite_layout: BindGroupLayout,
	sampler: Sampler,
	histogram: HashMap<(u32, CdfScope), CachedComputePipelineId>,
	composite: HashMap<TextureFormat, CachedRenderPipelineId>,
	// views render one after another so same sized ones share a per pixel histogram
	pixel: HashMap<(UVec2, u32), PixelHistogram>,
	// (material pipeline, bins, cdf scope) -> [prepass, accum], None when it isn't marked
	// for wboit
	derived: HashMap<(CachedRenderPipelineId, u32, CdfScope), Option<[CachedRenderPipelineId; 2]>>,
}
impl FromWorld for WboitPipelines {
	fn from_world(world: &mut World) -> Self {
		let device = world.resource::<RenderDevice>();
		let tex = || texture_2d(TextureSampleType::Float { filterable: true });
		let prepass_layout = device.create_bind_group_layout(
			"wboit_prepass",
			&BindGroupLayoutEntries::sequential(
				ShaderStages::FRAGMENT,
				(
					uniform_buffer::<WboitParams>(false),
					storage_buffer_sized(false, None),
					texture_depth_2d(),
				),
			),
		);
		let accum_layout = device.create_bind_group_layout(
			"wboit_accum",
			&BindGroupLayoutEntries::sequential(
				ShaderStages::FRAGMENT,
				(
					uniform_buffer::<WboitParams>(false),
					texture_2d_array(TextureSampleType::Float { filterable: true }),
					sampler(SamplerBindingType::Filtering),
					tex(),
					tex(),
					storage_buffer_read_only_sized(false, None),
				),
			),
		);
		let pixel_layout = device.create_bind_group_layout(
			"wboit_pixel",
			&BindGroupLayoutEntries::sequential(
				ShaderStages::FRAGMENT,
				(
					uniform_buffer::<WboitParams>(false),
					texture_2d_array(TextureSampleType::Float { filterable: false }),
				),
			),
		);
		let histogram_layout = device.create_bind_group_layout(
			"wboit_histogram",
			&BindGroupLayoutEntries::sequential(
				ShaderStages::COMPUTE,
				(
					uniform_buffer::<WboitParams>(false),
					storage_buffer_sized(false, None),
					texture_storage_2d_array(CDF, StorageTextureAccess::WriteOnly),
					storage_buffer_sized(false, None),
				),
			),
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
			prepass_layout,
			accum_layout,
			pixel_layout,
			histogram_layout,
			composite_layout,
			sampler,
			histogram: default(),
			composite: default(),
			pixel: default(),
			derived: default(),
		}
	}
}
impl WboitPipelines {
	fn histogram(&mut self, cache: &PipelineCache, bins: u32, scope: CdfScope) {
		if scope == CdfScope::Pixel {
			return;
		}
		let layout = self.histogram_layout.clone();
		self.histogram.entry((bins, scope)).or_insert_with(|| {
			cache.queue_compute_pipeline(ComputePipelineDescriptor {
				label: Some("wboit_histogram".into()),
				layout: vec![layout],
				push_constant_ranges: vec![],
				shader: HISTOGRAM_SHADER_HANDLE,
				shader_defs: vec![
					ShaderDefVal::UInt("WBOIT_BINS".into(), bins),
					ShaderDefVal::UInt("WBOIT_LAYERS".into(), bins / 4),
				],
				entry_point: match scope {
					CdfScope::Global => "reduce",
					_ => "resolve",
				}
				.into(),
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
		bins: u32,
		scope: CdfScope,
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
		let additive = BlendComponent {
			src_factor: BlendFactor::One,
			dst_factor: BlendFactor::One,
			operation: BlendOperation::Add,
		};
		let min = BlendComponent {
			operation: BlendOperation::Min,
			..additive
		};
		let target = |format, blend| {
			Some(ColorTargetState {
				format,
				blend: Some(BlendState {
					color: blend,
					alpha: blend,
				}),
				write_mask: ColorWrites::ALL,
			})
		};
		let variant = |pass: &'static str,
		               layout: &BindGroupLayout,
		               targets: Vec<Option<ColorTargetState>>| {
			let mut d = (**d).clone();
			d.label = Some(pass.to_lowercase().into());
			d.layout.push(layout.clone());
			let f = d.fragment.as_mut().unwrap();
			f.shader_defs
				.retain(|d| !matches!(d, ShaderDefVal::UInt(n, _) if n == "WBOIT_LAYERS"));
			f.shader_defs.extend([
				"WBOIT_PASS".into(),
				pass.into(),
				ShaderDefVal::UInt("WBOIT_BINS".into(), bins),
				ShaderDefVal::UInt("WBOIT_LAYERS".into(), bins / 4),
			]);
			match scope {
				CdfScope::Tiled => {}
				CdfScope::Global => f.shader_defs.push("WBOIT_GLOBAL".into()),
				CdfScope::Pixel => f.shader_defs.push("WBOIT_PIXEL".into()),
			}
			f.targets = targets;
			cache.queue_render_pipeline(d)
		};
		Some(Some(if scope == CdfScope::Pixel {
			[
				variant(
					"WBOIT_PREPASS",
					&self.prepass_layout,
					vec![target(PIXEL, additive); bins as usize / 4],
				),
				variant(
					"WBOIT_ACCUM",
					&self.pixel_layout,
					vec![
						target(TextureFormat::Rgba16Float, additive),
						target(TextureFormat::R16Float, additive),
					],
				),
			]
		} else {
			[
				variant(
					"WBOIT_PREPASS",
					&self.prepass_layout,
					vec![
						target(TextureFormat::R16Float, additive),
						target(TextureFormat::R16Float, min),
					],
				),
				variant(
					"WBOIT_ACCUM",
					&self.accum_layout,
					vec![target(TextureFormat::Rgba16Float, additive)],
				),
			]
		}))
	}
}

#[derive(Resource, Default, Deref, DerefMut)]
struct WboitViews(HashMap<RetainedViewEntity, ViewState>);

struct ViewState {
	wboit: Wboit,
	size: UVec2,
	tiles: UVec2,

	accum: TextureView,
	tau: TextureView,
	front: TextureView,
	hist: Buffer,
	params: UniformBuffer<WboitParams>,
	pixel: Option<PixelHistogram>,

	// rebuilt every frame since it holds the view's depth texture
	prepass: Option<BindGroup>,
	draw: BindGroup,
	histogram: BindGroup,
	composite: BindGroup,

	prepass_phase: SortedRenderPhase<Transparent3d>,
	accum_phase: SortedRenderPhase<Transparent3d>,
}
impl ViewState {
	fn new(
		device: &RenderDevice,
		queue: &RenderQueue,
		pipelines: &WboitPipelines,
		wboit: Wboit,
		size: UVec2,
		pixel: Option<PixelHistogram>,
	) -> Self {
		let tiles = (size + UVec2::splat(wboit.tile_size - 1)) / wboit.tile_size;
		let bins = wboit.bins.count();
		let texture = |label: &'static str,
		               size: UVec2,
		               format: TextureFormat,
		               layers: u32,
		               usage: TextureUsages| {
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
				usage: usage | TextureUsages::TEXTURE_BINDING,
				view_formats: &[],
			})
		};
		let tex = |label, format| {
			texture(label, size, format, 1, TextureUsages::RENDER_ATTACHMENT)
				.create_view(&default())
		};
		let accum = tex("wboit_accum", TextureFormat::Rgba16Float);
		let tau = tex("wboit_tau", TextureFormat::R16Float);
		let front = tex("wboit_front", TextureFormat::R16Float);
		let cdf = texture(
			"wboit_cdf",
			tiles,
			CDF,
			wboit.bins.layers(),
			TextureUsages::STORAGE_BINDING,
		)
		.create_view(&TextureViewDescriptor {
			dimension: Some(TextureViewDimension::D2Array),
			..default()
		});
		let buffer = |label, size: u32| {
			device.create_buffer(&BufferDescriptor {
				label: Some(label),
				size: size as u64 * 4,
				usage: BufferUsages::STORAGE,
				mapped_at_creation: false,
			})
		};
		let hist = buffer("wboit_hist", tiles.x * tiles.y * bins);
		let edges = buffer("wboit_global_edges", bins);

		let mut params = UniformBuffer::from(WboitParams {
			tiles,
			tile_size: wboit.tile_size,
			near: wboit.near,
			depth_scale: 1.0 / (wboit.far / wboit.near).ln(),
		});
		params.write_buffer(device, queue);

		let draw = match &pixel {
			Some(pixel) => device.create_bind_group(
				"wboit_pixel",
				&pipelines.pixel_layout,
				&BindGroupEntries::sequential((params.binding().unwrap(), &pixel.array)),
			),
			None => device.create_bind_group(
				"wboit_accum",
				&pipelines.accum_layout,
				&BindGroupEntries::sequential((
					params.binding().unwrap(),
					&cdf,
					&pipelines.sampler,
					&tau,
					&front,
					edges.as_entire_binding(),
				)),
			),
		};
		let histogram = device.create_bind_group(
			"wboit_histogram",
			&pipelines.histogram_layout,
			&BindGroupEntries::sequential((
				params.binding().unwrap(),
				hist.as_entire_binding(),
				&cdf,
				edges.as_entire_binding(),
			)),
		);
		let composite = device.create_bind_group(
			"wboit_composite",
			&pipelines.composite_layout,
			&BindGroupEntries::sequential((&accum, &tau)),
		);

		Self {
			wboit,
			size,
			tiles,
			accum,
			tau,
			front,
			hist,
			params,
			pixel,
			prepass: None,
			draw,
			histogram,
			composite,
			prepass_phase: default(),
			accum_phase: default(),
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
	let max_layers = limits
		.max_color_attachments
		.min(limits.max_color_attachment_bytes_per_sample / PIXEL.block_copy_size(None).unwrap());
	let mut pixels = HashSet::new();
	for (view, camera, target, wboit) in &cameras {
		let Some(size) = camera.physical_target_size else {
			continue;
		};
		let layers = wboit.bins.layers();
		if wboit.cdf == CdfScope::Pixel {
			if layers > max_layers {
				error_once!(
					"{:?} per pixel needs {layers} render targets but this device only allows {max_layers}, falling back to alpha blending",
					wboit.bins,
				);
				continue;
			}
			pixels.insert((size, layers));
		}
		live.insert(view.retained_view_entity);
		pipelines.composite(&cache, target.main_texture_format());
		pipelines.histogram(&cache, wboit.bins.count(), wboit.cdf);
		if !views
			.get(&view.retained_view_entity)
			.is_some_and(|s| s.size == size && s.wboit == *wboit)
		{
			let pixel = (wboit.cdf == CdfScope::Pixel).then(|| {
				pipelines
					.pixel
					.entry((size, layers))
					.or_insert_with(|| PixelHistogram::new(&device, size, layers))
					.clone()
			});
			let state = ViewState::new(&device, &queue, &pipelines, *wboit, size, pixel);
			views.insert(view.retained_view_entity, state);
		}
	}
	views.retain(|v, _| live.contains(v));
	pipelines.pixel.retain(|k, _| pixels.contains(k));
}

fn bind_prepass(
	mut views: ResMut<WboitViews>,
	depths: Query<(&ExtractedView, &ViewDepthTexture)>,
	pipelines: Res<WboitPipelines>,
	device: Res<RenderDevice>,
) {
	for (view, depth) in &depths {
		let Some(state) = views.get_mut(&view.retained_view_entity) else {
			continue;
		};
		state.prepass = Some(device.create_bind_group(
			"wboit_prepass",
			&pipelines.prepass_layout,
			&BindGroupEntries::sequential((
				state.params.binding().unwrap(),
				state.hist.as_entire_binding(),
				depth.view(),
			)),
		));
	}
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
		state.prepass_phase.items.clear();
		state.accum_phase.items.clear();
		let Some(phase) = phases.get_mut(view) else {
			continue;
		};
		let bins = state.wboit.bins.count();
		let scope = state.wboit.cdf;
		for item in mem::take(&mut phase.items) {
			let key = (item.pipeline, bins, scope);
			let derived = match pipelines.derived.get(&key) {
				Some(d) => *d,
				None => match pipelines.derive(&cache, item.pipeline, bins, scope) {
					Some(d) => *pipelines.derived.entry(key).or_insert(d),
					None => None,
				},
			};
			let Some([prepass, accum]) = derived else {
				phase.items.push(item);
				continue;
			};
			state.prepass_phase.add(Transparent3d {
				pipeline: prepass,
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
		let Some(prepass) = state.prepass.as_ref() else {
			return Ok(());
		};
		if state.accum_phase.items.is_empty() {
			return Ok(());
		}
		let pipelines = world.resource::<WboitPipelines>();
		let cache = world.resource::<PipelineCache>();
		let view_entity = graph.view_entity();
		// read only so the prepass can sample it at the same time
		let depth = || {
			Some(RenderPassDepthStencilAttachment {
				view: depth.view(),
				depth_ops: None,
				stencil_ops: None,
			})
		};

		{
			let mut pass = render_context.begin_tracked_render_pass(RenderPassDescriptor {
				label: Some("wboit_prepass"),
				color_attachments: &match &state.pixel {
					Some(pixel) => pixel
						.layers
						.iter()
						.map(|l| clear(l, LinearRgba::NONE))
						.collect::<Vec<_>>(),
					None => vec![
						clear(&state.tau, LinearRgba::NONE),
						clear(&state.front, LinearRgba::WHITE),
					],
				},
				depth_stencil_attachment: depth(),
				timestamp_writes: None,
				occlusion_query_set: None,
			});
			if let Some(viewport) = camera.viewport.as_ref() {
				pass.set_camera_viewport(viewport);
			}
			pass.set_bind_group(3, prepass, &[]);
			if let Err(err) = state.prepass_phase.render(&mut pass, world, view_entity) {
				error!("error rendering the wboit prepass: {err:?}");
			}
		}

		if let Some(histogram) = pipelines
			.histogram
			.get(&(state.wboit.bins.count(), state.wboit.cdf))
			.and_then(|id| cache.get_compute_pipeline(*id))
		{
			let mut pass =
				render_context
					.command_encoder()
					.begin_compute_pass(&ComputePassDescriptor {
						label: Some("wboit_histogram"),
						timestamp_writes: None,
					});
			pass.set_pipeline(histogram);
			pass.set_bind_group(0, &*state.histogram, &[]);
			match state.wboit.cdf {
				CdfScope::Tiled => {
					let groups = (state.tiles + UVec2::splat(7)) / 8;
					pass.dispatch_workgroups(groups.x, groups.y, 1);
				}
				CdfScope::Global => pass.dispatch_workgroups(1, 1, 1),
				CdfScope::Pixel => {}
			}
		}

		{
			let mut pass = render_context.begin_tracked_render_pass(RenderPassDescriptor {
				label: Some("wboit_accum"),
				color_attachments: &match state.pixel {
					Some(_) => vec![
						clear(&state.accum, LinearRgba::NONE),
						clear(&state.tau, LinearRgba::NONE),
					],
					None => vec![clear(&state.accum, LinearRgba::NONE)],
				},
				depth_stencil_attachment: depth(),
				timestamp_writes: None,
				occlusion_query_set: None,
			});
			if let Some(viewport) = camera.viewport.as_ref() {
				pass.set_camera_viewport(viewport);
			}
			pass.set_bind_group(3, &state.draw, &[]);
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
			pass.set_bind_group(0, &state.composite, &[]);
			pass.draw(0..3, 0..1);
		}

		Ok(())
	}
}

fn clear(view: &TextureView, color: LinearRgba) -> Option<RenderPassColorAttachment<'_>> {
	Some(RenderPassColorAttachment {
		view,
		resolve_target: None,
		ops: Operations {
			load: LoadOp::Clear(color.into()),
			store: StoreOp::Store,
		},
	})
}

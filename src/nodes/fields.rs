use crate::core::registry::Registry;
use crate::nodes::ProxyExt;
use crate::nodes::spatial::{Spatial, SpatialObject};
use crate::{DbusConnection, impl_proxy, interface};
use bevy::app::{Plugin, Update};
use bevy::asset::Assets;
use bevy::color::Color;
use bevy::ecs::component::Component;
use bevy::ecs::entity::Entity;
use bevy::ecs::query::With;
use bevy::ecs::resource::Resource;
use bevy::ecs::system::{Commands, Query, Res, ResMut};
use bevy::gizmos::GizmoAsset;
use bevy::gizmos::retained::Gizmo;
use glam::{Mat4, Vec3, Vec3A, vec3a};
use gluon_ipc::{Handler, RefExt};
use parking_lot::RwLock;
use stardust_xr_protocol::field::{
	CreatedField, Field as FieldProxy, FieldHandler, FieldInterfaceHandler,
	FieldRef as FieldRefProxy, FieldRefHandler, FieldSample, RayMarchResult, Shape,
};
use stardust_xr_protocol::spatial::{Spatial as SpatialProxy, SpatialRef as SpatialRefProxy};
use stardust_xr_protocol::types::{CreateError, Vec3F};
use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

pub struct FieldDebugGizmoPlugin;
impl Plugin for FieldDebugGizmoPlugin {
	fn build(&self, app: &mut bevy::app::App) {
		let (tx, rx) = tokio::sync::watch::channel(false);
		let conn = app.world().resource::<DbusConnection>().0.clone();
		tokio::spawn(async move {
			_ = conn
				.object_server()
				.at("/org/stardustxr/Server", FieldDebugGizmos { state: tx })
				.await;
		});
		app.insert_resource(FieldDebugGizmosEnabled(rx));
		app.init_resource::<FieldGizmoState>();
		app.add_systems(Update, sync_field_gizmos);
	}
}

#[derive(Resource)]
struct FieldDebugGizmosEnabled(tokio::sync::watch::Receiver<bool>);

#[derive(Component)]
struct FieldGizmoMarker;

#[derive(Resource, Default)]
struct FieldGizmoState(HashMap<usize, (u64, Vec<Entity>)>);

fn sync_field_gizmos(
	enabled: Res<FieldDebugGizmosEnabled>,
	mut commands: Commands,
	mut gizmo_assets: ResMut<Assets<GizmoAsset>>,
	mut state: ResMut<FieldGizmoState>,
	mut transforms: Query<&mut bevy::transform::components::Transform, With<FieldGizmoMarker>>,
) {
	if !*enabled.0.borrow() {
		for (_, (_, entities)) in state.0.drain() {
			for e in entities {
				commands.entity(e).despawn();
			}
		}
		return;
	}

	let fields = FIELD_REGISTRY_DEBUG_GIZMOS.get_valid_contents();
	let color = Color::srgb_u8(0x04, 0xFD, 0x4C);

	let alive_ptrs: HashSet<usize> = fields.iter().map(|f| Arc::as_ptr(f) as usize).collect();

	state.0.retain(|ptr, (_, entities)| {
		if alive_ptrs.contains(ptr) {
			true
		} else {
			for e in entities.drain(..) {
				commands.entity(e).despawn();
			}
			false
		}
	});

	for f in &fields {
		let ptr = Arc::as_ptr(f) as usize;
		let field_transform =
			bevy::transform::components::Transform::from_matrix(f.spatial.global_transform());
		let Some(cache) = f.polyline_cache.try_read() else {
			continue;
		};
		let current_gen = cache.0;

		let entry = state.0.entry(ptr).or_insert((u64::MAX, vec![]));

		if entry.0 == current_gen {
			for &e in &entry.1 {
				if let Ok(mut t) = transforms.get_mut(e) {
					*t = field_transform;
				}
			}
		} else if let Some(chains) = cache.1.as_ref() {
			for e in entry.1.drain(..) {
				commands.entity(e).despawn();
			}
			entry.0 = current_gen;

			for chain in chains {
				let mut asset = GizmoAsset::new();
				asset.linestrip(chain.iter().map(|c| (*c).into()), color);
				let handle = gizmo_assets.add(asset);
				let entity = commands
					.spawn((
						Gizmo {
							handle,
							..Default::default()
						},
						field_transform,
						FieldGizmoMarker,
					))
					.id();
				entry.1.push(entity);
			}
		}
		// else: generation changed but chains not ready yet — keep old entities visible
	}
}

/// Bails out early if `f` is the last strong reference remaining (i.e. every
/// `FieldObject`/`FieldRef` that pointed at it has already been dropped), since
/// that means the field is dead in all but name and there's no point spending
/// more time computing lines nobody will ever see — better to let it drop promptly.
fn compute_field_polylines(f: &Arc<Field>) -> Vec<Vec<Vec3A>> {
	const FAR: f32 = 100.0;
	const PAD: f32 = 1.1;
	const MIN_EXT: f32 = 0.005;
	let bx_pos = ((FAR - f.local_sample(vec3a(FAR, 0.0, 0.0)).distance) * PAD).max(MIN_EXT);
	let bx_neg = ((FAR - f.local_sample(vec3a(-FAR, 0.0, 0.0)).distance) * PAD).max(MIN_EXT);
	let by_pos = ((FAR - f.local_sample(vec3a(0.0, FAR, 0.0)).distance) * PAD).max(MIN_EXT);
	let by_neg = ((FAR - f.local_sample(vec3a(0.0, -FAR, 0.0)).distance) * PAD).max(MIN_EXT);
	let bz_pos = ((FAR - f.local_sample(vec3a(0.0, 0.0, FAR)).distance) * PAD).max(MIN_EXT);
	let bz_neg = ((FAR - f.local_sample(vec3a(0.0, 0.0, -FAR)).distance) * PAD).max(MIN_EXT);

	const MAX_SLICES_PER_HALF_AXIS: i32 = 10;
	let slice_step = 0.01_f32.max(
		[bx_neg, bx_pos, by_neg, by_pos, bz_neg, bz_pos]
			.into_iter()
			.fold(0.0_f32, f32::max)
			/ MAX_SLICES_PER_HALF_AXIS as f32,
	);
	let square_rez = 10.0;

	let slice_positions = |neg: f32, pos: f32| {
		let neg_count = (neg / slice_step).ceil() as i32;
		let pos_count = (pos / slice_step).ceil() as i32;
		(-neg_count..=pos_count).map(|i| i as f32 * slice_step)
	};

	let mut all_chains: Vec<Vec<Vec3A>> = Vec::new();

	for z in slice_positions(bz_neg, bz_pos).chain([-(bz_neg / PAD), bz_pos / PAD]) {
		if Arc::strong_count(f) <= 1 {
			return all_chains;
		}
		for chain in chain_segments(marching_squares_slice(
			|p| f.local_sample(p).distance,
			(-bx_neg, bx_pos, -by_neg, by_pos),
			slice_step / square_rez,
			move |u, v| vec3a(u, v, z),
		)) {
			all_chains.push(chain);
		}
	}

	for y in slice_positions(by_neg, by_pos).chain([-(by_neg / PAD), by_pos / PAD]) {
		if Arc::strong_count(f) <= 1 {
			return all_chains;
		}
		for chain in chain_segments(marching_squares_slice(
			|p| f.local_sample(p).distance,
			(-bx_neg, bx_pos, -bz_neg, bz_pos),
			slice_step / square_rez,
			move |u, v| vec3a(u, y, v),
		)) {
			all_chains.push(chain);
		}
	}

	for x in slice_positions(bx_neg, bx_pos).chain([-(bx_neg / PAD), bx_pos / PAD]) {
		if Arc::strong_count(f) <= 1 {
			return all_chains;
		}
		for chain in chain_segments(marching_squares_slice(
			|p| f.local_sample(p).distance,
			(-by_neg, by_pos, -bz_neg, bz_pos),
			slice_step / square_rez,
			move |u, v| vec3a(x, u, v),
		)) {
			all_chains.push(chain);
		}
	}

	all_chains
}

const RECALC_IDLE: u8 = 0;
const RECALC_RUNNING: u8 = 1;
const RECALC_RUNNING_DIRTY: u8 = 2;

/// Mirrors `FieldDebugGizmosEnabled` outside of Bevy so `request_field_polylines_recalc`
/// can skip the marching-squares pass entirely when nobody's watching the D-Bus viz.
static GIZMO_VIZ_ENABLED: AtomicBool = AtomicBool::new(false);

/// Request a recalculation of the field's debug polylines, coalescing requests that
/// arrive while a recalculation is already in flight instead of queuing up extra
/// blocking tasks (which would otherwise pile up and lag everything down if the
/// field's shape changes faster than the marching-squares pass can keep up).
fn request_field_polylines_recalc(field: &Arc<Field>) {
	if !GIZMO_VIZ_ENABLED.load(Ordering::Acquire) {
		return;
	}
	loop {
		match field.recalc_state.compare_exchange(
			RECALC_IDLE,
			RECALC_RUNNING,
			Ordering::AcqRel,
			Ordering::Acquire,
		) {
			Ok(_) => {
				let field = field.clone();
				tokio::task::spawn_blocking(move || run_field_polylines_recalc(&field));
				return;
			}
			Err(RECALC_RUNNING) => {
				match field.recalc_state.compare_exchange(
					RECALC_RUNNING,
					RECALC_RUNNING_DIRTY,
					Ordering::AcqRel,
					Ordering::Acquire,
				) {
					Ok(_) => return,
					Err(_) => continue,
				}
			}
			Err(_) => return, // already RECALC_RUNNING_DIRTY, nothing more to do
		}
	}
}

/// this needs to be called from a blocking context, else it panics
fn run_field_polylines_recalc(field: &Arc<Field>) {
	loop {
		spawn_field_polylines(field);
		match field.recalc_state.compare_exchange(
			RECALC_RUNNING,
			RECALC_IDLE,
			Ordering::AcqRel,
			Ordering::Acquire,
		) {
			Ok(_) => return,
			Err(_) => {
				// a request arrived mid-computation; recompute once more instead of
				// letting it queue up another task.
				field.recalc_state.store(RECALC_RUNNING, Ordering::Release);
				continue;
			}
		}
	}
}

fn spawn_field_polylines(field: &Arc<Field>) {
	let mut cache = field.polyline_cache.write();
	cache.0 += 1;
	let chains = compute_field_polylines(field);
	cache.1 = Some(chains);
}

/// Sample the SDF on a 2D grid over the given bounds and run standard marching squares
/// to find line segments at the zero isoline. Returns a list of (start, end) segment pairs
/// in local 3D space.
///
/// - `sample`: evaluates the SDF at a 3D local-space point
/// - `bounds`: (u_min, u_max, v_min, v_max) extent of the 2D grid
/// - `cell_size`: size of each grid cell
/// - `lift`: maps 2D (u, v) coordinates to a 3D local-space point on the slice plane
fn marching_squares_slice<S, L>(
	sample: S,
	bounds: (f32, f32, f32, f32),
	cell_size: f32,
	lift: L,
) -> Vec<(Vec3A, Vec3A)>
where
	S: Fn(Vec3A) -> f32,
	L: Fn(f32, f32) -> Vec3A,
{
	let (u_min, u_max, v_min, v_max) = bounds;
	let cols = ((u_max - u_min) / cell_size).ceil() as usize;
	let rows = ((v_max - v_min) / cell_size).ceil() as usize;
	if cols == 0 || rows == 0 {
		return vec![];
	}

	// Sample the SDF at each grid vertex: grid[row][col]
	let grid: Vec<Vec<f32>> = (0..=rows)
		.map(|j| {
			let v = v_min + j as f32 * cell_size;
			(0..=cols)
				.map(|i| {
					let u = u_min + i as f32 * cell_size;
					sample(lift(u, v))
				})
				.collect()
		})
		.collect();

	// Marching squares lookup table.
	// Corners per cell: 0=(i,j), 1=(i+1,j), 2=(i+1,j+1), 3=(i,j+1)
	// Edges: 0=bottom(c0-c1), 1=right(c1-c2), 2=top(c2-c3), 3=left(c3-c0)
	// Case index bit k is set when corner k is inside (d <= 0).
	const TABLE: [&[(usize, usize)]; 16] = [
		&[],               // 0000 – all outside
		&[(0, 3)],         // 0001 – c0
		&[(0, 1)],         // 0010 – c1
		&[(1, 3)],         // 0011 – c0 c1
		&[(1, 2)],         // 0100 – c2
		&[(0, 3), (1, 2)], // 0101 – c0 c2 (ambiguous)
		&[(0, 2)],         // 0110 – c1 c2
		&[(2, 3)],         // 0111 – c0 c1 c2
		&[(2, 3)],         // 1000 – c3
		&[(0, 2)],         // 1001 – c0 c3
		&[(0, 1), (2, 3)], // 1010 – c1 c3 (ambiguous)
		&[(1, 2)],         // 1011 – c0 c1 c3
		&[(1, 3)],         // 1100 – c2 c3
		&[(0, 1)],         // 1101 – c0 c2 c3
		&[(0, 3)],         // 1110 – c1 c2 c3
		&[],               // 1111 – all inside
	];

	let mut segments = Vec::new();

	for j in 0..rows {
		for i in 0..cols {
			let d0 = grid[j][i]; // c0: (i,   j  )
			let d1 = grid[j][i + 1]; // c1: (i+1, j  )
			let d2 = grid[j + 1][i + 1]; // c2: (i+1, j+1)
			let d3 = grid[j + 1][i]; // c3: (i,   j+1)

			let case_idx = ((d0 <= 0.0) as usize)
				| (((d1 <= 0.0) as usize) << 1)
				| (((d2 <= 0.0) as usize) << 2)
				| (((d3 <= 0.0) as usize) << 3);

			let entry = TABLE[case_idx];
			if entry.is_empty() {
				continue;
			}

			let u0 = u_min + i as f32 * cell_size;
			let v0 = v_min + j as f32 * cell_size;
			let u1 = u0 + cell_size;
			let v1 = v0 + cell_size;

			let c0: Vec3A = lift(u0, v0);
			let c1: Vec3A = lift(u1, v0);
			let c2: Vec3A = lift(u1, v1);
			let c3: Vec3A = lift(u0, v1);

			// Linearly interpolate to find the zero crossing on an edge.
			let edge_pt = |e: usize| -> Vec3A {
				let (ca, da, cb, db) = match e {
					0 => (c0, d0, c1, d1),
					1 => (c1, d1, c2, d2),
					2 => (c2, d2, c3, d3),
					_ => (c3, d3, c0, d0),
				};
				let denom = da - db;
				let t = if denom.abs() < 1e-10 { 0.5 } else { da / denom };
				ca.lerp(cb, t)
			};

			for &(ea, eb) in entry {
				segments.push((edge_pt(ea), edge_pt(eb)));
			}
		}
	}

	segments
}

/// Chain a list of `(start, end)` segment pairs into polylines by connecting shared endpoints.
/// Segments sharing an endpoint (within quantized precision) are merged into longer chains.
/// Closed loops are detected and the first point is appended to close them.
fn chain_segments(segments: Vec<(Vec3A, Vec3A)>) -> Vec<Vec<Vec3A>> {
	use std::collections::{HashMap, VecDeque};

	if segments.is_empty() {
		return vec![];
	}

	// Quantize a Vec3A to a (i32,i32,i32) key at 0.1 mm precision for endpoint matching.
	let quantize = |v: Vec3A| -> (i32, i32, i32) {
		(
			(v.x * 10_000.0) as i32,
			(v.y * 10_000.0) as i32,
			(v.z * 10_000.0) as i32,
		)
	};

	// Build adjacency map: endpoint key -> [(segment_idx, other_endpoint)]
	let mut adj: HashMap<(i32, i32, i32), Vec<(usize, Vec3A)>> = HashMap::new();
	for (idx, &(a, b)) in segments.iter().enumerate() {
		adj.entry(quantize(a)).or_default().push((idx, b));
		adj.entry(quantize(b)).or_default().push((idx, a));
	}

	let mut used = vec![false; segments.len()];
	let mut chains: Vec<Vec<Vec3A>> = Vec::new();

	for start in 0..segments.len() {
		if used[start] {
			continue;
		}
		used[start] = true;
		let (a, b) = segments[start];
		let mut chain: VecDeque<Vec3A> = vec![a, b].into();

		// Extend forward from tail.
		loop {
			let tail = *chain.back().unwrap();
			let found = adj
				.get(&quantize(tail))
				.and_then(|ns| ns.iter().find(|&&(idx, _)| !used[idx]).copied());
			let Some((idx, next)) = found else { break };
			used[idx] = true;
			// Close the loop if the next point rejoins the chain head.
			if quantize(next) == quantize(*chain.front().unwrap()) {
				chain.push_back(*chain.front().unwrap());
				break;
			}
			chain.push_back(next);
		}

		// Extend backward from head to catch segments that connect before the start.
		loop {
			let head = *chain.front().unwrap();
			let found = adj
				.get(&quantize(head))
				.and_then(|ns| ns.iter().find(|&&(idx, _)| !used[idx]).copied());
			let Some((idx, prev)) = found else { break };
			used[idx] = true;
			chain.push_front(prev);
		}

		chains.push(chain.into_iter().collect());
	}

	chains
}

struct FieldDebugGizmos {
	state: tokio::sync::watch::Sender<bool>,
}

#[zbus::interface(name = "org.stardustxr.debug.FieldDebugGizmos")]
impl FieldDebugGizmos {
	fn enable(&mut self) {
		GIZMO_VIZ_ENABLED.store(true, Ordering::Release);
		// polylines weren't kept up to date while disabled, so seed them now
		for field in FIELD_REGISTRY_DEBUG_GIZMOS.get_valid_contents() {
			request_field_polylines_recalc(&field);
		}
		_ = self.state.send(true);
	}
	fn disable(&mut self) {
		GIZMO_VIZ_ENABLED.store(false, Ordering::Release);
		_ = self.state.send(false);
	}
}

static FIELD_REGISTRY_DEBUG_GIZMOS: Registry<Field> = Registry::new();

pub struct Ray {
	pub origin: Vec3,
	pub direction: Vec3,
	pub space: Arc<Spatial>,
}

// const MIN_RAY_STEPS: u32 = 0;
const MAX_RAY_STEPS: u32 = 1000;

const MIN_RAY_MARCH: f32 = 0.001_f32;
const MAX_RAY_MARCH: f32 = f32::MAX;

// const MIN_RAY_LENGTH: f32 = 0_f32;
const MAX_RAY_LENGTH: f32 = 1000_f32;

pub struct ShapeChangedCallback(Arc<dyn Fn() + Send + Sync + 'static>);
impl Debug for ShapeChangedCallback {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_tuple("ShapeChangedCallback").finish()
	}
}
#[derive(Debug, Handler)]
pub struct FieldObject {
	pub data: Arc<Field>,
	field_ref: FieldRefProxy,
}
pub struct Field {
	pub spatial: Arc<Spatial>,
	pub shape: RwLock<Shape>,
	shape_changed_callback: Registry<dyn Fn() + Send + Sync + 'static>,
	polyline_cache: RwLock<(u64, Option<Vec<Vec<Vec3A>>>)>,
	recalc_state: AtomicU8,
}

impl Debug for Field {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Field")
			.field("spatial", &self.spatial)
			.field("shape", &self.shape)
			.field("polyline_cache", &self.polyline_cache)
			.finish()
	}
}
impl Field {
	#[cfg(test)]
	pub(crate) fn test_new(spatial: Arc<Spatial>, shape: Shape) -> Self {
		use crate::core::registry::Registry;
		Field {
			spatial,
			shape: RwLock::new(shape),
			shape_changed_callback: Registry::new(),
			polyline_cache: RwLock::new((0, None)),
			recalc_state: AtomicU8::new(RECALC_IDLE),
		}
	}

	pub fn shape_changed_callback(
		&self,
		f: impl Fn() + Send + Sync + 'static,
	) -> ShapeChangedCallback {
		let arc = ShapeChangedCallback(Arc::new(f));
		self.shape_changed_callback.add_raw(&arc.0);
		arc
	}

	pub fn local_sample(&self, p: Vec3A) -> FieldSample {
		self.shape.read().clone().sample(p)
	}
	pub fn sample(&self, reference_space: &Spatial, p: Vec3A) -> FieldSample {
		self.in_space(reference_space.global_transform().inverse())
			.sample(p)
	}

	pub fn ray_march(&self, ray: Ray) -> RayMarchResult {
		self.in_space(ray.space.global_transform().inverse())
			.ray_march(ray.origin, ray.direction, MAX_RAY_LENGTH)
	}

	/// relate this field to a space once, for sampling it many times
	pub fn in_space(&self, world_to_space: Mat4) -> FieldInSpace {
		let field_to_space = world_to_space * self.spatial.global_transform();
		let local = self.shape.read().clone();
		FieldInSpace {
			bound: shape_bound(&local).map(|(center, radius)| {
				(
					field_to_space.transform_point3a(center),
					radius * max_scale(&field_to_space),
				)
			}),
			local,
			space_to_field: field_to_space.inverse(),
			field_to_space,
		}
	}

	/// never more than the real distance from `p` in the field's own space
	pub fn local_distance_at_least(&self, p: Vec3A) -> f32 {
		shape_bound(&self.shape.read()).map_or(f32::NEG_INFINITY, |(center, radius)| {
			p.distance(center) - radius
		})
	}
}

/// a sphere around everything a shape can reach, as center and radius in the shape's space.
/// samples are never closer to `p` than `|p - center| - radius`, so anything farther than
/// that can be ruled out without sampling. none means no useful bound
fn shape_bound(shape: &Shape) -> Option<(Vec3A, f32)> {
	Some(match shape {
		Shape::Box { size } => (Vec3A::ZERO, Vec3A::from(Vec3::from(*size)).length() / 2.0),
		Shape::Sphere { radius } => (Vec3A::ZERO, *radius),
		Shape::Capsule { length, radius } => (Vec3A::ZERO, length / 2.0 + radius),
		Shape::Cylinder { length, radius } => (Vec3A::ZERO, (length / 2.0).hypot(*radius)),
		Shape::Torus {
			major_radius,
			minor_radius,
		} => (Vec3A::ZERO, major_radius + minor_radius),
		// handles might be relative, so no bound until that's certain
		Shape::CubicBezierSpline { .. } => return None,
		Shape::Transform { shape, transform } => {
			let m = Mat4::from(*transform);
			let (center, radius) = shape_bound(shape)?;
			(m.transform_point3a(center), radius * max_scale(&m))
		}
		Shape::Union { shapes } => enclosing(shapes)?,
		// blending pulls the surface out by at most k/4
		Shape::SmoothUnion { shapes, smoothing } => {
			let (center, radius) = enclosing(shapes)?;
			(center, radius + smoothing.abs() / 4.0)
		}
		Shape::Sweep { surface, sweeper } => {
			let (center, radius) = shape_bound(surface)?;
			let (sweep_center, sweep_radius) = shape_bound(sweeper)?;
			(center, radius + sweep_center.length() + sweep_radius)
		}
	})
}
fn enclosing(shapes: &[Shape]) -> Option<(Vec3A, f32)> {
	let bounds = shapes.iter().map(shape_bound).collect::<Option<Vec<_>>>()?;
	let (min, max) = bounds.iter().fold(
		(Vec3A::splat(f32::MAX), Vec3A::splat(f32::MIN)),
		|(lo, hi), (c, r)| (lo.min(c - r), hi.max(c + r)),
	);
	let center = (min + max) / 2.0;
	let radius = bounds
		.iter()
		.map(|(c, r)| center.distance(*c) + r)
		.reduce(f32::max)?;
	Some((center, radius))
}
fn max_scale(m: &Mat4) -> f32 {
	m.x_axis
		.truncate()
		.length()
		.max(m.y_axis.truncate().length())
		.max(m.z_axis.truncate().length())
}

/// a field as seen from one space, so repeated samples skip walking parent chains,
/// cloning the shape and inverting matrices every time
pub struct FieldInSpace {
	local: Shape,
	bound: Option<(Vec3A, f32)>,
	field_to_space: Mat4,
	space_to_field: Mat4,
}
impl FieldInSpace {
	// the same steps as sampling a `Shape::Transform`, with the inverse worked out once
	pub fn sample(&self, p: Vec3A) -> FieldSample {
		let local = self.local.sample(self.space_to_field.transform_point3a(p));
		let closest = self
			.field_to_space
			.transform_point3a(local.closest_point.into());
		FieldSample {
			distance: local.distance.signum() * (p - closest).length(),
			gradient: self
				.space_to_field
				.transpose()
				.transform_vector3(local.gradient.into())
				.normalize_or_zero()
				.into(),
			closest_point: closest.into(),
		}
	}

	/// never more than the real distance, so far away points can be ruled out cheaply
	pub fn distance_at_least(&self, p: Vec3A) -> f32 {
		self.bound.map_or(f32::NEG_INFINITY, |(center, radius)| {
			p.distance(center) - radius
		})
	}

	/// never more than the closest a ray gets to the field within `max_length`
	pub fn ray_distance_at_least(&self, origin: Vec3A, direction: Vec3A, max_length: f32) -> f32 {
		let Some((center, radius)) = self.bound else {
			return f32::NEG_INFINITY;
		};
		let along = direction.length_squared();
		let t = if along > 0.0 {
			((center - origin).dot(direction) / along).clamp(0.0, max_length.min(MAX_RAY_LENGTH))
		} else {
			0.0
		};
		(origin + direction * t).distance(center) - radius
	}

	// `sample(p).distance` without the gradient, for ray marching
	fn distance(&self, p: Vec3A) -> f32 {
		let local = self.local.sample(self.space_to_field.transform_point3a(p));
		let closest = self
			.field_to_space
			.transform_point3a(local.closest_point.into());
		local.distance.signum() * (p - closest).length()
	}

	/// stops at `max_length`, past that nothing the caller cares about can be found
	pub fn ray_march(&self, mut origin: Vec3, direction: Vec3, max_length: f32) -> RayMarchResult {
		let max_length = max_length.min(MAX_RAY_LENGTH);
		let mut result = RayMarchResult {
			min_distance: f32::MAX,
			deepest_point_distance: 0_f32,
			ray_length: 0_f32,
			ray_steps: 0,
		};

		while result.ray_steps < MAX_RAY_STEPS && result.ray_length < max_length {
			let distance = self.distance(origin.into());
			let march_distance = distance.clamp(MIN_RAY_MARCH, MAX_RAY_MARCH);

			result.ray_length += march_distance;
			origin += direction * march_distance;

			if result.min_distance > distance {
				result.deepest_point_distance = result.ray_length;
				result.min_distance = distance;
			}

			result.ray_steps += 1;
		}

		result
	}
}
impl FieldObject {
	pub fn new(
		spatial: Arc<SpatialObject>,
		shape: Shape,
	) -> gluon_ipc::LocalRef<FieldProxy, FieldObject> {
		let data = Arc::new(Field {
			spatial: spatial.spatial_arc().clone(),
			shape: RwLock::new(shape),
			polyline_cache: RwLock::new((0, None)),
			shape_changed_callback: Registry::new(),
			recalc_state: AtomicU8::new(RECALC_IDLE),
		});
		FIELD_REGISTRY_DEBUG_GIZMOS.add_raw(&data);
		request_field_polylines_recalc(&data);
		// TODO: remove those unwraps
		let field_ref = FieldRefProxy::new_service(FieldRef { data: data.clone() })
			.unwrap()
			.into_proxy();
		FieldProxy::new_service(FieldObject { field_ref, data }).unwrap()
	}
}
impl Drop for Field {
	fn drop(&mut self) {
		FIELD_REGISTRY_DEBUG_GIZMOS.remove(self);
	}
}
impl FieldObject {
	pub fn get_ref(&self) -> &FieldRefProxy {
		&self.field_ref
	}
	pub fn set_shape(&self, shape: Shape) {
		*self.data.shape.write() = shape;
		request_field_polylines_recalc(&self.data);
		for f in self.data.shape_changed_callback.get_valid_contents() {
			f()
		}
	}
}
impl FieldHandler for FieldObject {
	async fn field_ref(&self, _ctx: gluon_ipc::Context) -> FieldRefProxy {
		self.field_ref.clone()
	}

	async fn sample(
		&self,
		_ctx: gluon_ipc::Context,
		reference_space: SpatialRefProxy,
		point: Vec3F,
	) -> FieldSample {
		let Some(ref_space) = reference_space.owned() else {
			return FieldSample::infinite();
		};
		self.data.sample(&ref_space, point.into())
	}

	async fn ray_march(
		&self,
		_ctx: gluon_ipc::Context,
		reference_space: SpatialRefProxy,
		ray_origin: Vec3F,
		ray_direction: Vec3F,
	) -> Option<RayMarchResult> {
		let ref_space = reference_space.owned()?;
		Some(self.data.ray_march(Ray {
			origin: ray_origin.into(),
			direction: ray_direction.into(),
			space: (**ref_space).clone(),
		}))
	}

	async fn set_shape(&self, _ctx: gluon_ipc::Context, shape: Shape) {
		FieldObject::set_shape(self, shape);
	}
}

#[derive(Debug, Handler)]
pub struct FieldRef {
	pub data: Arc<Field>,
}
impl FieldRefHandler for FieldRef {}

interface!(FieldInterface);
impl FieldInterfaceHandler for FieldInterface {
	async fn sample(
		&self,
		_ctx: gluon_ipc::Context,
		field: FieldRefProxy,
		space: SpatialRefProxy,
		point: Vec3F,
	) -> FieldSample {
		let Some(field) = field.owned() else {
			return FieldSample::infinite();
		};
		let Some(space) = space.owned() else {
			return FieldSample::infinite();
		};
		let point = point.into();
		field.data.sample(&space, point)
	}

	async fn ray_march(
		&self,
		_ctx: gluon_ipc::Context,
		field: FieldRefProxy,
		space: SpatialRefProxy,
		ray_origin: Vec3F,
		ray_direction: Vec3F,
	) -> Option<RayMarchResult> {
		let space = space.owned()?;
		let field = field.owned()?;
		Some(field.data.ray_march(Ray {
			origin: ray_origin.into(),
			direction: ray_direction.into(),
			space: (**space).clone(),
		}))
	}

	async fn create_field(
		&self,
		_ctx: gluon_ipc::Context,
		spatial: SpatialProxy,
		shape: Shape,
	) -> Result<CreatedField, CreateError> {
		let spatial = spatial.owned().ok_or(CreateError::InvalidRef)?;
		let field = FieldObject::new(spatial, shape);
		Ok(CreatedField {
			field_ref: field.get_ref().clone(),
			field: field.into_proxy(),
		})
	}
}

impl_proxy!(FieldProxy, FieldObject);
impl_proxy!(FieldRefProxy, FieldRef);

#[cfg(test)]
mod tests {
	use super::*;
	use crate::core::registry::Registry;
	use crate::nodes::spatial::Spatial;
	use glam::{Mat4, Vec3, vec3a};
	use stardust_xr_protocol::field::Shape;
	use stardust_xr_protocol::types::Vec3F;

	fn approx_eq(a: f32, b: f32) -> bool {
		(a - b).abs() < 1e-4
	}

	fn make_field(spatial: Arc<Spatial>, shape: Shape) -> Field {
		Field {
			spatial,
			shape: RwLock::new(shape),
			shape_changed_callback: Registry::new(),
			polyline_cache: RwLock::new((0, None)),
			recalc_state: AtomicU8::new(RECALC_IDLE),
		}
	}

	fn origin_spatial() -> Arc<Spatial> {
		Spatial::test_new(None, Mat4::IDENTITY)
	}

	fn translated_spatial(x: f32, y: f32, z: f32) -> Arc<Spatial> {
		Spatial::test_new(None, Mat4::from_translation(Vec3::new(x, y, z)))
	}

	#[test]
	fn local_sample_sphere_center_is_inside() {
		let field = make_field(origin_spatial(), Shape::Sphere { radius: 1.0 });
		let sample = field.local_sample(vec3a(0.0, 0.0, 0.0));
		assert!(
			approx_eq(sample.distance, -1.0),
			"expected -1.0, got {}",
			sample.distance
		);
	}

	#[test]
	fn local_sample_sphere_exterior() {
		let field = make_field(origin_spatial(), Shape::Sphere { radius: 1.0 });
		let sample = field.local_sample(vec3a(3.0, 0.0, 0.0));
		assert!(
			approx_eq(sample.distance, 2.0),
			"expected 2.0, got {}",
			sample.distance
		);
	}

	#[test]
	fn local_sample_sphere_surface() {
		let field = make_field(origin_spatial(), Shape::Sphere { radius: 1.0 });
		let sample = field.local_sample(vec3a(1.0, 0.0, 0.0));
		assert!(
			approx_eq(sample.distance, 0.0),
			"expected 0.0, got {}",
			sample.distance
		);
	}

	#[test]
	fn local_sample_box_center() {
		let field = make_field(
			origin_spatial(),
			Shape::Box {
				size: Vec3F {
					x: 2.0,
					y: 2.0,
					z: 2.0,
				},
			},
		);
		let sample = field.local_sample(vec3a(0.0, 0.0, 0.0));
		assert!(
			approx_eq(sample.distance, -1.0),
			"expected -1.0, got {}",
			sample.distance
		);
	}

	#[test]
	fn local_sample_box_exterior() {
		let field = make_field(
			origin_spatial(),
			Shape::Box {
				size: Vec3F {
					x: 2.0,
					y: 2.0,
					z: 2.0,
				},
			},
		);
		let sample = field.local_sample(vec3a(3.0, 0.0, 0.0));
		assert!(
			approx_eq(sample.distance, 2.0),
			"expected 2.0, got {}",
			sample.distance
		);
	}

	#[test]
	fn sample_colocated_spaces_matches_local_sample() {
		let field_spatial = origin_spatial();
		let ref_spatial = origin_spatial();
		let field = make_field(field_spatial, Shape::Sphere { radius: 1.0 });

		let p = vec3a(2.0, 0.0, 0.0);
		let local = field.local_sample(p);
		let cross = field.sample(&ref_spatial, p);
		assert!(
			approx_eq(local.distance, cross.distance),
			"colocated spaces: local={} cross={}",
			local.distance,
			cross.distance
		);
	}

	#[test]
	fn sample_field_offset_point_at_field_center() {
		// Field sphere at world (5, 0, 0); sample point (5, 0, 0) in reference
		// (world-origin) space → local position is (0, 0, 0) → inside, distance -1.
		let field_spatial = translated_spatial(5.0, 0.0, 0.0);
		let ref_spatial = origin_spatial();
		let field = make_field(field_spatial, Shape::Sphere { radius: 1.0 });

		let sample = field.sample(&ref_spatial, vec3a(5.0, 0.0, 0.0));
		assert!(
			approx_eq(sample.distance, -1.0),
			"expected -1.0, got {}",
			sample.distance
		);
	}

	#[test]
	fn sample_field_offset_point_outside() {
		// Field sphere at world (5, 0, 0), radius 1; sample at (8, 0, 0) in
		// reference space → 3 units from center → distance 2.0.
		let field_spatial = translated_spatial(5.0, 0.0, 0.0);
		let ref_spatial = origin_spatial();
		let field = make_field(field_spatial, Shape::Sphere { radius: 1.0 });

		let sample = field.sample(&ref_spatial, vec3a(8.0, 0.0, 0.0));
		assert!(
			approx_eq(sample.distance, 2.0),
			"expected 2.0, got {}",
			sample.distance
		);
	}

	#[test]
	fn sample_reference_offset_from_field() {
		// Field at world origin, sphere radius 1; reference at (3, 0, 0).
		// Point (0, 0, 0) in reference space = (3, 0, 0) in world → distance 2.0.
		let field_spatial = origin_spatial();
		let ref_spatial = translated_spatial(3.0, 0.0, 0.0);
		let field = make_field(field_spatial, Shape::Sphere { radius: 1.0 });

		let sample = field.sample(&ref_spatial, vec3a(0.0, 0.0, 0.0));
		assert!(
			approx_eq(sample.distance, 2.0),
			"expected 2.0, got {}",
			sample.distance
		);
	}

	#[test]
	fn sample_parented_reference_space() {
		// Reference child at local (2, 0, 0) under a parent at (3, 0, 0);
		// effective world position = (5, 0, 0). Field sphere at origin, radius 1.
		// Point (0, 0, 0) in reference child space = (5, 0, 0) in world → distance 4.0.
		let parent_spatial = translated_spatial(3.0, 0.0, 0.0);
		let child_spatial = Spatial::test_new(
			Some(parent_spatial),
			Mat4::from_translation(Vec3::new(2.0, 0.0, 0.0)),
		);
		let field_spatial = origin_spatial();
		let field = make_field(field_spatial, Shape::Sphere { radius: 1.0 });

		let sample = field.sample(&child_spatial, vec3a(0.0, 0.0, 0.0));
		assert!(
			approx_eq(sample.distance, 4.0),
			"expected 4.0, got {}",
			sample.distance
		);
	}

	// sampling through a precomputed space must land on exactly what the protocol's own
	// transformed shape gives, gradient and closest point included
	#[test]
	fn in_space_matches_protocol_sample() {
		let parent = Spatial::test_new(
			None,
			Mat4::from_scale_rotation_translation(
				Vec3::new(1.0, 2.0, 0.5),
				glam::Quat::from_rotation_y(0.7),
				Vec3::new(1.0, -2.0, 3.0),
			),
		);
		let reference = Spatial::test_new(Some(parent), Mat4::from_rotation_x(0.3));
		let field = make_field(
			translated_spatial(-1.0, 0.5, 2.0),
			Shape::Box {
				size: [0.4, 1.0, 0.2].into(),
			},
		);
		let in_space = field.in_space(reference.global_transform().inverse());
		let protocol = Shape::Transform {
			shape: Box::new(field.shape.read().clone()),
			transform: Spatial::space_to_space_matrix(Some(&field.spatial), Some(&reference))
				.into(),
		};
		for p in [
			vec3a(0.0, 0.0, 0.0),
			vec3a(-2.0, 1.0, 0.5),
			vec3a(3.0, -1.0, -4.0),
		] {
			let expected = protocol.sample(p);
			assert_eq!(in_space.sample(p), expected);
			assert_eq!(in_space.distance(p), expected.distance);
		}
	}

	// skipping far queryables is only exact if the bound never overshoots a real sample
	#[test]
	fn bound_never_exceeds_sampled_distance() {
		let squish = Mat4::from_scale_rotation_translation(
			Vec3::new(2.0, 0.3, 1.0),
			glam::Quat::from_rotation_z(0.8),
			Vec3::new(0.5, -1.0, 0.2),
		);
		let shapes = [
			Shape::Box {
				size: [1.0, 0.4, 2.0].into(),
			},
			Shape::Sphere { radius: 0.7 },
			Shape::Capsule {
				length: 1.5,
				radius: 0.3,
			},
			Shape::Cylinder {
				length: 1.0,
				radius: 0.6,
			},
			Shape::Torus {
				major_radius: 1.0,
				minor_radius: 0.2,
			},
			Shape::Transform {
				shape: Box::new(Shape::Box {
					size: [1.0, 1.0, 1.0].into(),
				}),
				transform: squish.into(),
			},
			Shape::Union {
				shapes: vec![
					Shape::Sphere { radius: 0.3 },
					Shape::Transform {
						shape: Box::new(Shape::Sphere { radius: 0.5 }),
						transform: Mat4::from_translation(Vec3::new(2.0, 0.0, 0.0)).into(),
					},
				],
			},
			Shape::SmoothUnion {
				shapes: vec![
					Shape::Sphere { radius: 0.4 },
					Shape::Transform {
						shape: Box::new(Shape::Sphere { radius: 0.4 }),
						transform: Mat4::from_translation(Vec3::new(0.6, 0.0, 0.0)).into(),
					},
				],
				smoothing: 0.5,
			},
			Shape::Sweep {
				surface: Box::new(Shape::Box {
					size: [1.0, 1.0, 1.0].into(),
				}),
				sweeper: Box::new(Shape::Sphere { radius: 0.25 }),
			},
		];
		let mut seed = 0x9E37_79B9_u32;
		let mut next = || {
			seed ^= seed << 13;
			seed ^= seed >> 17;
			seed ^= seed << 5;
			seed as f32 / u32::MAX as f32 * 8.0 - 4.0
		};
		let field = make_field(
			Spatial::test_new(Some(translated_spatial(1.0, 2.0, 3.0)), squish),
			Shape::Torus {
				major_radius: 1.0,
				minor_radius: 0.3,
			},
		);
		let in_space = field.in_space(
			translated_spatial(-2.0, 0.0, 1.0)
				.global_transform()
				.inverse(),
		);
		for shape in &shapes {
			let (center, radius) = shape_bound(shape).expect("these shapes all have bounds");
			for _ in 0..3000 {
				let p = vec3a(next(), next(), next());
				let sampled = shape.sample(p).distance;
				assert!(
					p.distance(center) - radius <= sampled + 1e-4,
					"{shape:?} at {p}: bound {} over sample {sampled}",
					p.distance(center) - radius
				);
				assert!(in_space.distance_at_least(p) <= in_space.sample(p).distance + 1e-4);
			}
		}
	}
}

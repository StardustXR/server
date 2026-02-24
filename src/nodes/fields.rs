use super::alias::{Alias, AliasInfo};
use super::spatial::{
	SPATIAL_REF_GET_LOCAL_BOUNDING_BOX_SERVER_OPCODE,
	SPATIAL_REF_GET_RELATIVE_BOUNDING_BOX_SERVER_OPCODE, SPATIAL_REF_GET_TRANSFORM_SERVER_OPCODE,
	Spatial,
};
use super::{Aspect, AspectIdentifier, Node};
use crate::DbusConnection;
use crate::core::Id;
use crate::core::client::Client;
use crate::core::error::Result;
use crate::core::registry::Registry;
use crate::nodes::spatial::SPATIAL_ASPECT_ALIAS_INFO;
use crate::nodes::spatial::SPATIAL_REF_ASPECT_ALIAS_INFO;
use crate::nodes::spatial::Transform;
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
use color_eyre::eyre::OptionExt;
use dashmap::DashMap;
use glam::{Vec3, Vec3A, Vec3Swizzles, vec2, vec3, vec3a};
use parking_lot::Mutex;
use stardust_xr_wire::values::Vector3;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Weak};
use zbus::interface;

// TODO: get SDFs working properly with non-uniform scale and so on, output distance relative to the spatial it's compared against

pub static FIELD_ALIAS_INFO: LazyLock<AliasInfo> = LazyLock::new(|| AliasInfo {
	server_methods: vec![
		SPATIAL_REF_GET_TRANSFORM_SERVER_OPCODE,
		SPATIAL_REF_GET_LOCAL_BOUNDING_BOX_SERVER_OPCODE,
		SPATIAL_REF_GET_RELATIVE_BOUNDING_BOX_SERVER_OPCODE,
		FIELD_REF_DISTANCE_SERVER_OPCODE,
		FIELD_REF_NORMAL_SERVER_OPCODE,
		FIELD_REF_CLOSEST_POINT_SERVER_OPCODE,
		FIELD_REF_RAY_MARCH_SERVER_OPCODE,
	],
	..Default::default()
});

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
		let cache = f.polyline_cache.lock();
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
				asset.linestrip(chain.iter().copied(), color);
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

fn compute_field_polylines(f: &Field) -> Vec<Vec<Vec3>> {
	const FAR: f32 = 100.0;
	const PAD: f32 = 1.1;
	const MIN_EXT: f32 = 0.005;
	let bx_pos = ((FAR - f.local_distance(vec3a(FAR, 0.0, 0.0))) * PAD).max(MIN_EXT);
	let bx_neg = ((FAR - f.local_distance(vec3a(-FAR, 0.0, 0.0))) * PAD).max(MIN_EXT);
	let by_pos = ((FAR - f.local_distance(vec3a(0.0, FAR, 0.0))) * PAD).max(MIN_EXT);
	let by_neg = ((FAR - f.local_distance(vec3a(0.0, -FAR, 0.0))) * PAD).max(MIN_EXT);
	let bz_pos = ((FAR - f.local_distance(vec3a(0.0, 0.0, FAR))) * PAD).max(MIN_EXT);
	let bz_neg = ((FAR - f.local_distance(vec3a(0.0, 0.0, -FAR))) * PAD).max(MIN_EXT);

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

	let mut all_chains: Vec<Vec<Vec3>> = Vec::new();

	for z in slice_positions(bz_neg, bz_pos).chain([-(bz_neg / PAD), bz_pos / PAD]) {
		for chain in chain_segments(marching_squares_slice(
			|p| f.local_distance(p),
			(-bx_neg, bx_pos, -by_neg, by_pos),
			slice_step / square_rez,
			move |u, v| vec3a(u, v, z),
		)) {
			all_chains.push(chain);
		}
	}

	for y in slice_positions(by_neg, by_pos).chain([-(by_neg / PAD), by_pos / PAD]) {
		for chain in chain_segments(marching_squares_slice(
			|p| f.local_distance(p),
			(-bx_neg, bx_pos, -bz_neg, bz_pos),
			slice_step / square_rez,
			move |u, v| vec3a(u, y, v),
		)) {
			all_chains.push(chain);
		}
	}

	for x in slice_positions(bx_neg, bx_pos).chain([-(bx_neg / PAD), bx_pos / PAD]) {
		for chain in chain_segments(marching_squares_slice(
			|p| f.local_distance(p),
			(-by_neg, by_pos, -bz_neg, bz_pos),
			slice_step / square_rez,
			move |u, v| vec3a(x, u, v),
		)) {
			all_chains.push(chain);
		}
	}

	all_chains
}

fn spawn_field_polylines(field: Arc<Field>) {
	let generation = {
		let mut cache = field.polyline_cache.lock();
		cache.0 += 1;
		cache.1 = None;
		cache.0
	};
	tokio::task::spawn_blocking(move || {
		let chains = compute_field_polylines(&field);
		let mut cache = field.polyline_cache.lock();
		if cache.0 == generation {
			cache.1 = Some(chains);
		}
	});
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
) -> Vec<(Vec3, Vec3)>
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

			let c0: Vec3 = lift(u0, v0).into();
			let c1: Vec3 = lift(u1, v0).into();
			let c2: Vec3 = lift(u1, v1).into();
			let c3: Vec3 = lift(u0, v1).into();

			// Linearly interpolate to find the zero crossing on an edge.
			let edge_pt = |e: usize| -> Vec3 {
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
fn chain_segments(segments: Vec<(Vec3, Vec3)>) -> Vec<Vec<Vec3>> {
	use std::collections::{HashMap, VecDeque};

	if segments.is_empty() {
		return vec![];
	}

	// Quantize a Vec3 to a (i32,i32,i32) key at 0.1 mm precision for endpoint matching.
	let quantize = |v: Vec3| -> (i32, i32, i32) {
		(
			(v.x * 10_000.0) as i32,
			(v.y * 10_000.0) as i32,
			(v.z * 10_000.0) as i32,
		)
	};

	// Build adjacency map: endpoint key -> [(segment_idx, other_endpoint)]
	let mut adj: HashMap<(i32, i32, i32), Vec<(usize, Vec3)>> = HashMap::new();
	for (idx, &(a, b)) in segments.iter().enumerate() {
		adj.entry(quantize(a)).or_default().push((idx, b));
		adj.entry(quantize(b)).or_default().push((idx, a));
	}

	let mut used = vec![false; segments.len()];
	let mut chains: Vec<Vec<Vec3>> = Vec::new();

	for start in 0..segments.len() {
		if used[start] {
			continue;
		}
		used[start] = true;
		let (a, b) = segments[start];
		let mut chain: VecDeque<Vec3> = vec![a, b].into();

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

#[interface(name = "org.stardustxr.debug.FieldDebugGizmos")]
impl FieldDebugGizmos {
	fn enable(&mut self) {
		_ = self.state.send(true);
	}
	fn disable(&mut self) {
		_ = self.state.send(false);
	}
}

static FIELD_REGISTRY_DEBUG_GIZMOS: Registry<Field> = Registry::new();

stardust_xr_server_codegen::codegen_field_protocol!();

impl CubicSplineShape {
	/// Iterate over cubic Bezier segments as (P0, P1, P2, P3, r0, r3).
	fn segments(&self) -> impl Iterator<Item = (Vec3, Vec3, Vec3, Vec3, f32, f32)> + '_ {
		let n = self.control_points.len();
		let count = if self.cyclic { n } else { n.saturating_sub(1) };

		(0..count).map(move |i| {
			let a = &self.control_points[i];
			let b = &self.control_points[(i + 1) % n];
			(
				a.anchor.into(),
				a.handle_out.into(),
				b.handle_in.into(),
				b.anchor.into(),
				a.thickness,
				b.thickness,
			)
		})
	}

	/// Evaluate cubic Bezier curve at parameter t.
	fn eval_cubic(p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3, t: f32) -> Vec3 {
		let it = 1.0 - t;
		let it2 = it * it;
		let it3 = it2 * it;
		let t2 = t * t;
		let t3 = t2 * t;
		it3 * p0 + 3.0 * it2 * t * p1 + 3.0 * it * t2 * p2 + t3 * p3
	}

	/// First derivative of cubic Bezier at parameter t.
	fn eval_cubic_deriv(p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3, t: f32) -> Vec3 {
		let it = 1.0 - t;
		3.0 * it * it * (p1 - p0) + 6.0 * it * t * (p2 - p1) + 3.0 * t * t * (p3 - p2)
	}

	/// Second derivative of cubic Bezier at parameter t.
	fn eval_cubic_deriv2(p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3, t: f32) -> Vec3 {
		6.0 * (1.0 - t) * (p2 - 2.0 * p1 + p0) + 6.0 * t * (p3 - 2.0 * p2 + p1)
	}

	/// Find the parameter t that minimizes distance from point x to the cubic
	/// Bezier curve (p0, p1, p2, p3) using multi-start Newton iteration on
	/// the derivative of squared distance.
	fn closest_t_on_cubic(p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3, x: Vec3) -> f32 {
		let mut best_t = 0.0_f32;
		let mut best_dist_sq = f32::INFINITY;

		// Coarse sampling to find a good starting region
		const N_SAMPLES: usize = 8;
		for i in 0..=N_SAMPLES {
			let t = i as f32 / N_SAMPLES as f32;
			let pt = Self::eval_cubic(p0, p1, p2, p3, t);
			let d2 = (pt - x).length_squared();
			if d2 < best_dist_sq {
				best_dist_sq = d2;
				best_t = t;
			}
		}

		// Newton iteration: find root of f(t) = (B(t)-x)·B'(t)
		// using f'(t) = B'(t)·B'(t) + (B(t)-x)·B''(t)
		let mut t = best_t;
		for _ in 0..8 {
			let b = Self::eval_cubic(p0, p1, p2, p3, t);
			let b1 = Self::eval_cubic_deriv(p0, p1, p2, p3, t);
			let b2 = Self::eval_cubic_deriv2(p0, p1, p2, p3, t);

			let diff = b - x;
			let f = diff.dot(b1);
			let f_prime = b1.dot(b1) + diff.dot(b2);

			if f_prime.abs() < 1e-12 {
				break;
			}
			let dt = f / f_prime;
			t = (t - dt).clamp(0.0, 1.0);
			if dt.abs() < 1e-8 {
				break;
			}
		}

		t
	}

	/// SDF of the spline as a solid tube with per-control-point radii.
	/// Uses true cubic Bezier closest-point via Newton iteration.
	pub fn sd_tube(&self, p: Vec3) -> f32 {
		if self.control_points.len() < 2 {
			return f32::INFINITY;
		}

		self.segments()
			.map(|(p0, p1, p2, p3, r0, r3)| {
				let t = Self::closest_t_on_cubic(p0, p1, p2, p3, p);
				let closest = Self::eval_cubic(p0, p1, p2, p3, t);
				let radius = r0 + (r3 - r0) * t;
				(p - closest).length() - radius
			})
			.fold(f32::INFINITY, f32::min)
	}
}

pub static EXPORTED_FIELDS: LazyLock<DashMap<u64, Weak<Node>>> = LazyLock::new(DashMap::new);

pub trait FieldTrait: Send + Sync + 'static {
	fn spatial_ref(&self) -> &Spatial;

	fn local_distance(&self, p: Vec3A) -> f32;
	fn local_normal(&self, p: Vec3A, r: f32) -> Vec3A {
		let d = self.local_distance(p);
		let e = vec2(r, 0_f32);

		let n = vec3a(d, d, d)
			- vec3a(
				self.local_distance(vec3a(e.x, e.y, e.y)),
				self.local_distance(vec3a(e.y, e.x, e.y)),
				self.local_distance(vec3a(e.y, e.y, e.x)),
			);

		n.normalize()
	}
	fn local_closest_point(&self, p: Vec3A, r: f32) -> Vec3A {
		p - (self.local_normal(p, r) * self.local_distance(p))
	}

	fn distance(&self, reference_space: &Spatial, p: Vec3A) -> f32 {
		let reference_to_local_space =
			Spatial::space_to_space_matrix(Some(reference_space), Some(self.spatial_ref()));
		let local_p = reference_to_local_space.transform_point3a(p);
		self.local_distance(local_p)
	}
	fn normal(&self, reference_space: &Spatial, p: Vec3A, r: f32) -> Vec3A {
		let reference_to_local_space =
			Spatial::space_to_space_matrix(Some(reference_space), Some(self.spatial_ref()));
		let local_p = reference_to_local_space.transform_point3a(p);
		reference_to_local_space
			.inverse()
			.transform_vector3a(self.local_normal(local_p, r))
	}
	fn closest_point(&self, reference_space: &Spatial, p: Vec3A, r: f32) -> Vec3A {
		let reference_to_local_space =
			Spatial::space_to_space_matrix(Some(reference_space), Some(self.spatial_ref()));
		let local_p = reference_to_local_space.transform_point3a(p);
		reference_to_local_space
			.inverse()
			.transform_point3a(self.local_closest_point(local_p, r))
	}

	fn ray_march(&self, ray: Ray) -> RayMarchResult {
		let mut result = RayMarchResult {
			ray_origin: ray.origin.into(),
			ray_direction: ray.direction.into(),
			min_distance: f32::MAX,
			deepest_point_distance: 0_f32,
			ray_length: 0_f32,
			ray_steps: 0,
		};

		let ray_to_field_matrix =
			Spatial::space_to_space_matrix(Some(&ray.space), Some(self.spatial_ref()));
		let mut ray_point = ray_to_field_matrix.transform_point3a(ray.origin.into());
		let ray_direction = ray_to_field_matrix
			.transform_vector3a(ray.direction.into())
			.normalize();

		while result.ray_steps < MAX_RAY_STEPS && result.ray_length < MAX_RAY_LENGTH {
			let distance = self.local_distance(ray_point);
			let march_distance = distance.clamp(MIN_RAY_MARCH, MAX_RAY_MARCH);

			result.ray_length += march_distance;
			ray_point += ray_direction * march_distance;

			if result.min_distance > distance {
				result.deepest_point_distance = result.ray_length;
				result.min_distance = distance;
			}

			result.ray_steps += 1;
		}

		result
	}
}

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

pub struct Field {
	pub spatial: Arc<Spatial>,
	pub shape: Mutex<Shape>,
	polyline_cache: Mutex<(u64, Option<Vec<Vec<Vec3>>>)>,
}
impl Field {
	pub fn add_to(node: &Arc<Node>, shape: Shape) -> Result<Arc<Field>> {
		let spatial = node.get_aspect::<Spatial>()?;
		let field = Field {
			spatial,
			shape: Mutex::new(shape),
			polyline_cache: Mutex::new((0, None)),
		};
		let field = node.add_aspect(field);
		FIELD_REGISTRY_DEBUG_GIZMOS.add_raw(&field);
		spawn_field_polylines(field.clone());
		node.add_aspect(FieldRef);
		Ok(field)
	}
}
impl Drop for Field {
	fn drop(&mut self) {
		FIELD_REGISTRY_DEBUG_GIZMOS.remove(self);
	}
}
impl AspectIdentifier for Field {
	impl_aspect_for_field_aspect_id! {}
}
impl Aspect for Field {
	impl_aspect_for_field_aspect! {}
}
impl FieldAspect for Field {
	fn set_shape(node: Arc<Node>, _calling_client: Arc<Client>, shape: Shape) -> Result<()> {
		let field = node.get_aspect::<Field>()?;
		*field.shape.lock() = shape;
		spawn_field_polylines(field.clone());
		Ok(())
	}

	async fn export_field(node: Arc<Node>, _calling_client: Arc<Client>) -> Result<Id> {
		let id = rand::random();
		EXPORTED_FIELDS.insert(id, Arc::downgrade(&node));
		Ok(id.into())
	}
}
impl FieldTrait for Field {
	fn spatial_ref(&self) -> &Spatial {
		&self.spatial
	}
	fn local_distance(&self, p: Vec3A) -> f32 {
		match self.shape.lock().clone() {
			Shape::Box(size) => {
				let q = vec3(
					p.x.abs() - (size.x * 0.5_f32),
					p.y.abs() - (size.y * 0.5_f32),
					p.z.abs() - (size.z * 0.5_f32),
				);
				let v = vec3a(q.x.max(0_f32), q.y.max(0_f32), q.z.max(0_f32));
				v.length() + q.x.max(q.y.max(q.z)).min(0_f32)
			}
			Shape::Cylinder(CylinderShape { length, radius }) => {
				let d = vec2(p.xz().length().abs() - radius, p.y.abs() - (length * 0.5));
				d.x.max(d.y).min(0.0) + d.max(vec2(0.0, 0.0)).length()
			}
			Shape::Sphere(radius) => p.length() - radius,
			Shape::Spline(spline) => spline.sd_tube(p.into()),
			Shape::Torus(TorusShape { radius_a, radius_b }) => {
				let q = vec2(p.xz().length() - radius_a, p.y);
				q.length() - radius_b
			}
		}
	}
}

pub struct FieldRef;
impl AspectIdentifier for FieldRef {
	impl_aspect_for_field_ref_aspect_id! {}
}
impl Aspect for FieldRef {
	impl_aspect_for_field_ref_aspect! {}
}
impl FieldRefAspect for FieldRef {
	async fn distance(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		space: Arc<Node>,
		point: Vector3<f32>,
	) -> Result<f32> {
		let reference_space = space.get_aspect::<Spatial>()?;
		let field = node.get_aspect::<Field>()?;
		Ok(field.distance(&reference_space, point.into()))
	}

	async fn normal(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		space: Arc<Node>,
		point: Vector3<f32>,
	) -> Result<Vector3<f32>> {
		let reference_space = space.get_aspect::<Spatial>()?;
		let field = node.get_aspect::<Field>()?;
		Ok(field.normal(&reference_space, point.into(), 0.0001).into())
	}

	async fn closest_point(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		space: Arc<Node>,
		point: Vector3<f32>,
	) -> Result<Vector3<f32>> {
		let reference_space = space.get_aspect::<Spatial>()?;
		let field = node.get_aspect::<Field>()?;
		Ok(field
			.closest_point(&reference_space, point.into(), 0.0001)
			.into())
	}

	async fn ray_march(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		space: Arc<Node>,
		ray_origin: Vector3<f32>,
		ray_direction: Vector3<f32>,
	) -> Result<RayMarchResult> {
		let space = space.get_aspect::<Spatial>()?;
		let field = node.get_aspect::<Field>()?;
		Ok(field.ray_march(Ray {
			origin: ray_origin.into(),
			direction: ray_direction.into(),
			space,
		}))
	}
}

impl InterfaceAspect for Interface {
	async fn import_field_ref(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		uid: Id,
	) -> Result<Id> {
		let node = EXPORTED_FIELDS
			.get(&uid.0)
			.and_then(|s| s.upgrade())
			.map(|s| {
				Alias::create(
					&s,
					&calling_client,
					FIELD_REF_ASPECT_ALIAS_INFO.clone(),
					None,
				)
				.unwrap()
			})
			.ok_or_eyre("Couldn't import field with that ID")?;
		Ok(node.get_id())
	}

	fn create_field(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		id: Id,
		parent: Arc<Node>,
		transform: Transform,
		shape: Shape,
	) -> Result<()> {
		let transform = transform.to_mat4(true, true, false);
		let parent = parent.get_aspect::<Spatial>()?;
		let node = Node::from_id(&calling_client, id, true).add_to_scenegraph()?;
		Spatial::add_to(&node, Some(parent.clone()), transform);
		Field::add_to(&node, shape)?;
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn make_spline(points: &[([f32; 3], [f32; 3], [f32; 3], f32)], cyclic: bool) -> CubicSplineShape {
		CubicSplineShape {
			control_points: points
				.iter()
				.map(|(hi, a, ho, t)| CubicControlPoint {
					handle_in: mint::Vector3::from(*hi),
					anchor: mint::Vector3::from(*a),
					handle_out: mint::Vector3::from(*ho),
					thickness: *t,
				})
				.collect(),
			cyclic,
		}
	}

	#[test]
	fn sd_tube_straight_line() {
		let spline = make_spline(
			&[
				([0.0, 0.0, 0.0], [0.0, 0.0, 0.0], [0.33, 0.0, 0.0], 0.05),
				([0.67, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 0.0, 0.0], 0.05),
			],
			false,
		);

		let d = spline.sd_tube(Vec3::new(0.5, 0.05, 0.0));
		eprintln!("surface point: {d}");
		assert!(d.abs() < 0.01, "expected ~0 at surface, got {d}");

		let d = spline.sd_tube(Vec3::new(0.5, 0.0, 0.0));
		eprintln!("inside point: {d}");
		assert!(d < 0.0, "expected negative inside tube, got {d}");

		let d = spline.sd_tube(Vec3::new(0.5, 0.2, 0.0));
		eprintln!("outside point: {d}");
		assert!(d > 0.0, "expected positive outside tube, got {d}");

		let d = spline.sd_tube(Vec3::new(0.0, 0.0, 0.0));
		eprintln!("endpoint: {d}");
		assert!(d < 0.0, "expected negative at anchor, got {d}");
	}

	#[test]
	fn sd_tube_curved() {
		let spline = make_spline(
			&[
				([0.0, 0.0, 0.0], [0.0, 0.0, 0.0], [0.1, 0.2, 0.0], 0.02),
				([0.2, 0.2, 0.0], [0.3, 0.0, 0.0], [0.3, 0.0, 0.0], 0.02),
			],
			false,
		);

		for i in 0..=10 {
			let t = i as f32 / 10.0;
			let curve_pt = CubicSplineShape::eval_cubic(
				Vec3::new(0.0, 0.0, 0.0),
				Vec3::new(0.1, 0.2, 0.0),
				Vec3::new(0.2, 0.2, 0.0),
				Vec3::new(0.3, 0.0, 0.0),
				t,
			);
			let d = spline.sd_tube(curve_pt);
			eprintln!("t={t:.1} curve_pt={curve_pt} sd={d}");
			assert!(d < 0.0, "point on curve should be inside tube, t={t}, sd={d}");
		}
	}
}

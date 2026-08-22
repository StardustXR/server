use crate::{
	BevyMaterial,
	bevy_int::{color::ColorConvert, entity_handle::EntityHandle},
	core::{error::Result, registry::Registry},
	interface,
	nodes::{
		ProxyExt,
		spatial::{BoundingBoxCalc, SpatialObject},
	},
};
use bevy::{
	asset::{AssetEvents, RenderAssetUsages, weak_handle},
	pbr::{ExtendedMaterial, MaterialExtension},
	prelude::*,
	render::{
		mesh::{Indices, PrimitiveTopology, VertexAttributeValues},
		primitives::Aabb,
		render_resource::{AsBindGroup, ShaderRef},
		view::VisibilitySystems,
	},
};
use glam::Vec3;
use gluon::{Handler, LocalRef, RefExt};
use parking_lot::Mutex;
use stardust_xr_protocol::lines::{Line, LinePoint, LinesHandler, LinesInterfaceHandler};
use stardust_xr_protocol::{lines::Lines as LinesProxy, types::CreateError};
use std::sync::{
	Arc, OnceLock, Weak,
	atomic::{AtomicBool, Ordering},
};
use tokio::sync::Notify;

type LineMaterial = ExtendedMaterial<BevyMaterial, LineExtension>;
const LINE_SHADER_HANDLE: Handle<Shader> = weak_handle!("7d28aa5a-3abd-43bb-b0e9-0de8b81b650d");
// No extra data needed for a simple holdout
#[derive(Default, Asset, AsBindGroup, TypePath, Debug, Clone)]
#[data(50, u32, binding_array(101))]
#[bindless(index_table(range(50..51), binding(100)))]
pub struct LineExtension {}
impl From<&LineExtension> for u32 {
	fn from(_: &LineExtension) -> Self {
		0
	}
}
impl MaterialExtension for LineExtension {
	fn fragment_shader() -> ShaderRef {
		LINE_SHADER_HANDLE.into()
	}

	fn prepass_fragment_shader() -> ShaderRef {
		LINE_SHADER_HANDLE.into()
	}

	fn deferred_fragment_shader() -> ShaderRef {
		LINE_SHADER_HANDLE.into()
	}

	fn alpha_mode() -> Option<AlphaMode> {
		Some(AlphaMode::Blend)
	}
}

pub struct LinesNodePlugin;
impl Plugin for LinesNodePlugin {
	fn build(&self, app: &mut App) {
		app.add_systems(
			PostUpdate,
			(build_line_mesh, update_line_vis)
				.chain()
				.after(TransformSystem::TransformPropagate)
				.before(AssetEvents)
				.after(VisibilitySystems::VisibilityPropagate)
				.before(VisibilitySystems::CheckVisibility),
		);
		app.world_mut().resource_mut::<Assets<Shader>>().insert(
			LINE_SHADER_HANDLE.id(),
			Shader::from_wgsl(
				include_str!("line.wgsl"),
				std::path::Path::new(file!())
					.parent()
					.unwrap()
					.join("line.wgsl")
					.to_string_lossy(),
			),
		);
		app.add_plugins(MaterialPlugin::<LineMaterial>::default());
	}
}

#[derive(Component)]
struct LinesNode(Weak<Lines>);

fn update_line_vis(
	query: Query<(&InheritedVisibility, &LinesNode), Changed<InheritedVisibility>>,
	mut cmds: Commands,
) {
	for (vis, line) in &query {
		let Some(line) = line.0.upgrade() else {
			continue;
		};
		let Some(target) = line.entity.get() else {
			continue;
		};

		cmds.entity(target.entity())
			.insert(*vis)
			.insert(match vis.get() {
				true => Visibility::Visible,
				false => Visibility::Hidden,
			});
	}
}

fn build_line_mesh(
	mut cmds: Commands,
	mut meshes: ResMut<Assets<Mesh>>,
	mut materials: ResMut<Assets<LineMaterial>>,
	query: Query<Ref<GlobalTransform>>,
) {
	for lines in LINES_REGISTRY.get_valid_contents().into_iter() {
		let Some(transform) = lines.spatial.get_entity().and_then(|e| query.get(e).ok()) else {
			continue;
		};
		if !(lines.gen_mesh.load(Ordering::Relaxed) || transform.is_changed()) {
			continue;
		}
		lines.gen_mesh.store(false, Ordering::Relaxed);
		let mut vertex_positions = Vec::<Vec3>::new();
		let mut vertex_normals = Vec::<Vec3>::new();
		let mut vertex_colors = Vec::<[f32; 4]>::new();
		let mut vertex_indices = Vec::<u32>::new();
		let lines_data = lines.data.lock();
		if lines_data.is_empty() {
			*lines.bounds.lock() = Some(Aabb::default());
			lines.setup_complete.notify_waiters();
			match lines.entity.get() {
				Some(e) => cmds.entity(**e),
				None => {
					// if we couldn't get the lines entity then we need to gen the mesh later
					lines.gen_mesh.store(true, Ordering::Relaxed);
					continue;
				}
			}
			.remove::<Mesh3d>();
			continue;
		}

		let mut indices_set = 0;
		for line in lines_data.iter() {
			// yes this alloc is suboptimal, but good enough for now
			let line_points = line
				.points
				.iter()
				.map(|p: &LinePoint| LinePoint {
					point: transform.transform_point(p.point.into()).into(),
					thickness: p.thickness,
					color: p.color,
				})
				// Drop genuinely non-finite points (NaN/inf coming from the client
				// or a degenerate transform) so they can't poison the tube math.
				// This is distinct from coincident points, which we keep below.
				.filter(|p| Vec3::from(p.point).is_finite())
				.collect::<Vec<_>>();

			let start_set = indices_set;
			let n = line_points.len();
			// A line needs at least two points to form a tube.
			if n < 2 {
				continue;
			}

			// Direction of the segment *entering* point `i` (curr - prev), using the
			// nearest preceding point that is at a *different* position. Coincident
			// points (same position, authored to make the tube step in thickness or
			// colour) are stepped over rather than skipped, so we never normalize a
			// zero-length segment and every point in a coincident run ends up with
			// the same orientation/normal.
			let incoming_dir = |i: usize| -> Option<Vec3> {
				let curr = Vec3::from(line_points[i].point);
				(1..n).find_map(|steps| {
					let j = if line.cyclic {
						(i + n - steps) % n
					} else if i >= steps {
						i - steps
					} else {
						return None;
					};
					let prev = Vec3::from(line_points[j].point);
					(prev != curr).then(|| (curr - prev).normalize())
				})
			};
			// Direction of the segment *leaving* point `i` (next - curr), using the
			// nearest following point at a different position.
			let outgoing_dir = |i: usize| -> Option<Vec3> {
				let curr = Vec3::from(line_points[i].point);
				(1..n).find_map(|steps| {
					let j = if line.cyclic {
						(i + steps) % n
					} else if i + steps < n {
						i + steps
					} else {
						return None;
					};
					let next = Vec3::from(line_points[j].point);
					(next != curr).then(|| (next - curr).normalize())
				})
			};

			for (i, curr) in line_points.iter().enumerate() {
				let last_quat = incoming_dir(i).map(|d| Quat::from_rotation_arc(Vec3::Y, d));
				let next_quat = outgoing_dir(i).map(|d| Quat::from_rotation_arc(Vec3::Y, d));
				let quat = match (last_quat, next_quat) {
					// No distinct neighbour in either direction => every point in this
					// line shares one position, so there is no tube to orient.
					(None, None) => {
						error!("degenerate line: all points coincident");
						break;
					}
					(None, Some(q)) | (Some(q), None) => q,
					(Some(last), Some(next)) => last.lerp(next, 0.5),
				};
				if !quat.is_finite() {
					error!("non finite quat at point {i}: curr: {curr:?}");
					break;
				}
				let normals = [
					Vec3::X,
					Vec3::new(1., 0., 1.).normalize(),
					Vec3::Z,
					Vec3::new(-1., 0., 1.).normalize(),
					Vec3::NEG_X,
					Vec3::new(-1., 0., -1.).normalize(),
					Vec3::NEG_Z,
					Vec3::new(1., 0., -1.).normalize(),
				]
				.map(Vec3::normalize)
				.map(|v| quat * v);
				let points = normals.map(|v| (v * curr.thickness) + Vec3::from(curr.point));
				vertex_normals.extend(normals);
				vertex_positions.extend(points);
				vertex_colors.extend([curr.color.to_bevy().to_linear().to_f32_array(); 8]);
				// Connect this ring forward to the next one, except at the final
				// point of the line: a non-cyclic line is closed off by the caps and
				// a cyclic line is wrapped by cyclic_indices, both added below.
				let last_point = i == n - 1;
				if !last_point {
					vertex_indices.extend(indices(indices_set));
				}
				indices_set += 1;
			}
			// Only finish the tube if this line actually emitted any rings.
			if indices_set > start_set {
				// Handle the connection between start and end points:
				// - For cyclic lines: connect last segment back to first
				// - For non-cyclic lines: add caps at both ends
				if line.cyclic {
					vertex_indices.extend(cyclic_indices(start_set, indices_set - 1));
				} else {
					vertex_indices.extend(cap_indices(start_set, false));
					vertex_indices.extend(cap_indices(indices_set - 1, true));
				}
			}
		}
		let mut mesh = Mesh::new(
			PrimitiveTopology::TriangleList,
			RenderAssetUsages::RENDER_WORLD,
		);
		mesh.insert_indices(Indices::U32(vertex_indices));
		mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vertex_positions);
		mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vertex_normals);
		mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vertex_colors);

		let mut entity = match lines.entity.get() {
			Some(e) => cmds.entity(**e),
			None => {
				let ent = cmds
					.spawn((
						Name::new("LinesNodeProxy"),
						ChildOf(lines.spatial.get_entity().unwrap()),
						LinesNode(Arc::downgrade(&lines)),
					))
					.id();
				_ = lines.spatial_child_entity.set(EntityHandle::new(ent));
				let e = cmds.spawn((
					Name::new("LinesNode"),
					MeshMaterial3d(materials.add(ExtendedMaterial {
						base: BevyMaterial {
							base_color: Color::WHITE,
							perceptual_roughness: 1.0,
							alpha_mode: AlphaMode::Premultiplied,
							emissive: Color::linear_rgba(0.75, 0.75, 0.75, 1.0).into(),
							..default()
						},
						extension: LineExtension {},
					})),
				));
				_ = lines.entity.set(EntityHandle::new(e.id()));

				e
			}
		};
		if let Some(VertexAttributeValues::Float32x3(values)) =
			mesh.attribute(Mesh::ATTRIBUTE_POSITION)
		{
			let global_to_local = transform.affine().inverse();
			let local_aabb = Aabb::enclosing(
				values
					.iter()
					.map(|p| global_to_local.transform_point3(Vec3::from_slice(p))),
			)
			.unwrap_or_default();
			let global_aabb =
				Aabb::enclosing(values.iter().map(|p| Vec3::from_slice(p))).unwrap_or_default();
			*lines.bounds.lock() = Some(local_aabb);
			lines.setup_complete.notify_waiters();
			entity.insert(global_aabb);
		}
		entity.insert(Mesh3d(meshes.add(mesh)));
	}
}

const END_CAP_INDICES: [u32; 18] = [0, 1, 7, 7, 1, 2, 7, 2, 6, 6, 2, 3, 6, 3, 5, 5, 3, 4];
fn cap_indices(set: u32, flip: bool) -> [u32; END_CAP_INDICES.len()] {
	let mut out = END_CAP_INDICES.map(|v| v + (set * 8));
	if flip {
		out.reverse();
	}
	out
}

// const BASE: [u16; 6] = [0, 8, 1, 8, 9, 1];

// Defines how vertices are connected between consecutive cross-sections to form the tube
const INDICES: [u32; 48] = [
	0, 8, 1, 8, 9, 1, 1, 9, 2, 9, 10, 2, 2, 10, 3, 10, 11, 3, 3, 11, 4, 11, 12, 4, 4, 12, 5, 12,
	13, 5, 5, 13, 6, 13, 14, 6, 6, 14, 7, 14, 15, 7, 7, 15, 0, 15, 8, 0,
];
fn indices(set: u32) -> [u32; INDICES.len()] {
	INDICES.map(|v| v + (set * 8))
}
fn cyclic_indices(start_set: u32, end_set: u32) -> [u32; INDICES.len()] {
	let mut out = INDICES.map(|v| {
		if v < 8 {
			v + ((start_set) * 8)
		} else {
			v + ((end_set - 1) * 8)
		}
	});
	out.reverse();
	out
}

static LINES_REGISTRY: Registry<Lines> = Registry::new();

#[derive(Debug, Handler)]
pub struct Lines {
	spatial: Arc<SpatialObject>,
	data: Mutex<Vec<Line>>,
	gen_mesh: AtomicBool,
	entity: OnceLock<EntityHandle>,
	spatial_child_entity: OnceLock<EntityHandle>,
	bounds: Mutex<Option<Aabb>>,
	_bounding_calc: BoundingBoxCalc,
	setup_complete: Notify,
}
impl Lines {
	pub fn new(spatial: Arc<SpatialObject>, lines: Vec<Line>) -> LocalRef<LinesProxy, Lines> {
		let lines = Arc::new_cyclic(|weak: &Weak<Lines>| {
			let weak = weak.clone();
			let bounding_calc = spatial.custom_bounding_box(move || {
				let Some(lines) = weak.upgrade() else {
					return Default::default();
				};
				lines.bounds.lock().unwrap_or_default()
			});
			Lines {
				spatial: spatial.clone(),
				data: Mutex::new(lines),
				gen_mesh: AtomicBool::new(true),
				entity: OnceLock::new(),
				spatial_child_entity: OnceLock::new(),
				bounds: Mutex::new(Some(Aabb::default())),
				_bounding_calc: bounding_calc,
				setup_complete: Notify::new(),
			}
		});

        // TODO: get rid of this unwrap
		let lines = LinesProxy::new_service(lines).unwrap();
		let lines_arc = lines.handler().clone();
		LINES_REGISTRY.add_raw(&lines_arc);

		lines
	}
}
impl LinesHandler for Lines {
	async fn set_lines(&self, _ctx: gluon::Context, lines: Vec<Line>) {
		*self.data.lock() = lines;
		self.gen_mesh.store(true, Ordering::Relaxed);
	}
}
impl Drop for Lines {
	fn drop(&mut self) {
		LINES_REGISTRY.remove(self);
	}
}
interface!(LinesInterface);
impl LinesInterfaceHandler for LinesInterface {
	async fn create_lines(
		&self,
		_ctx: gluon::Context,
		spatial: stardust_xr_protocol::spatial::Spatial,
		lines: Vec<Line>,
	) -> Result<LinesProxy, CreateError> {
		let spatial = spatial.owned().ok_or(CreateError::InvalidRef)?;
		let lines = Lines::new(spatial.clone(), lines);
		tracing::info!("creating lines node");
		lines.setup_complete.notified().await;
		Ok(lines.into_proxy())
	}
}

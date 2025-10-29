use super::{Line, LinesAspect};
use crate::{
	BevyMaterial,
	core::{
		client::Client, color::ColorConvert, entity_handle::EntityHandle, error::Result,
		registry::Registry,
	},
	nodes::{
		Node,
		drawable::LinePoint,
		spatial::{Spatial, SpatialNode},
	},
};
use bevy::{
	asset::{AssetEvents, RenderAssetUsages, weak_handle},
	pbr::{ExtendedMaterial, MaterialExtension},
	prelude::*,
	render::{
		mesh::{Indices, MeshAabb, PrimitiveTopology},
		primitives::Aabb,
		render_resource::{AsBindGroup, ShaderRef},
		view::VisibilitySystems,
	},
};
use glam::Vec3;
use parking_lot::Mutex;
use std::sync::{
	Arc, OnceLock,
	atomic::{AtomicBool, Ordering},
};

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
			build_line_mesh
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

fn build_line_mesh(
	mut cmds: Commands,
	mut meshes: ResMut<Assets<Mesh>>,
	mut materials: ResMut<Assets<LineMaterial>>,
	query: Query<(&GlobalTransform, &InheritedVisibility)>,
) {
	for lines in LINES_REGISTRY
		.get_valid_contents()
		.into_iter()
		// .filter(|l| l.gen_mesh.load(Ordering::Relaxed))
	{
		lines.gen_mesh.store(false, Ordering::Relaxed);
		let mut vertex_positions = Vec::<Vec3>::new();
		let mut vertex_normals = Vec::<Vec3>::new();
		let mut vertex_colors = Vec::<[f32; 4]>::new();
		let mut vertex_indices = Vec::<u32>::new();
		let lines_data = lines.data.lock();
		let Some((transform, visibil)) = lines.spatial.get_entity().and_then(|e| query.get(e).ok())
		else {
			continue;
		};
		if lines_data.is_empty() {
			*lines.bounds.lock() = Aabb::default();
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
					// point: transform.transform_point(p.point.into()).into(),
					point: transform.transform_point(p.point.into()).into(),
					thickness: p.thickness,
					color: p.color,
				})
				.collect::<Vec<_>>();

			let start_set = indices_set;
			// Create a sliding window of points to process each segment of the line
			// For cyclic lines: wraps around by connecting last point back to first
			// For non-cyclic lines: handles endpoints with None values
			let point_windows = {
				let mut out = Vec::new();
				let mut last = line.cyclic.then(|| line_points.last()).flatten();
				let mut peekable = line_points.iter().peekable();
				while let Some(curr) = peekable.next() {
					// Skip this point if it has the same position as the previous point
					if let Some(prev) = last
						&& Vec3::from(prev.point) == Vec3::from(curr.point)
					{
						last = Some(curr);
						continue;
					}

					let mut end = false;
					// Determine the next point - either the next in sequence or
					// for cyclic lines, wrap back to first point at the end
					let next = match peekable.peek() {
						Some(v) => Some(*v),
						None => {
							end = true;
							line.cyclic.then(|| line_points.first()).flatten()
						}
					};

					out.push((last, curr, next, end));
					last = Some(curr);
				}
				out
			};
			// if we can't make a full line, don't bother trying
			if point_windows.len() < 2 {
				continue;
			}
			for (last, curr, next, last_point) in point_windows {
				let last_quat = last.map(|v| {
					Quat::from_rotation_arc(
						Vec3::Y,
						(Vec3::from(curr.point) - Vec3::from(v.point)).normalize(),
					)
				});
				let next_quat = next.map(|v| {
					Quat::from_rotation_arc(
						Vec3::Y,
						(Vec3::from(v.point) - Vec3::from(curr.point)).normalize(),
					)
				});
				let quat = match (last_quat, next_quat) {
					(None, None) => {
						error!("no previous or next point in line");
						break;
					}
					(None, Some(next)) => next,
					(Some(last), None) => last,
					(Some(last), Some(next)) => last.lerp(next, 0.5),
				};
				if !quat.is_finite() {
					error!("non finite quat: next: {next:?}, last: {last:?}, curr: {curr:?},");
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
				// Only connect vertices between segments if this isn't the end point
				if !last_point {
					vertex_indices.extend(indices(indices_set));
				}
				indices_set += 1;
			}
			if indices_set > 0 {
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
				let e = cmds.spawn((
					Name::new("LinesNode"),
					SpatialNode(Arc::downgrade(&lines.spatial)),
					MeshMaterial3d(materials.add(ExtendedMaterial {
						base: BevyMaterial {
							base_color: Color::WHITE,
							perceptual_roughness: 1.0,
							alpha_mode: AlphaMode::Premultiplied,
							emissive: Color::linear_rgba(0.25, 0.25, 0.25, 1.0).into(),
							..default()
						},
						extension: LineExtension {},
					})),
				));
				_ = lines.entity.set(EntityHandle::new(e.id()));
				e
			}
		};
		if let Some(aabb) = mesh.compute_aabb() {
			*lines.bounds.lock() = aabb;
			entity.insert(aabb);
		}
		entity
			.insert(Mesh3d(meshes.add(mesh)))
			.insert(*visibil)
			.insert(match visibil.get() {
				true => Visibility::Visible,
				false => Visibility::Hidden,
			});
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

pub struct Lines {
	spatial: Arc<Spatial>,
	data: Mutex<Vec<Line>>,
	gen_mesh: AtomicBool,
	entity: OnceLock<EntityHandle>,
	bounds: Mutex<Aabb>,
}
impl Lines {
	pub fn add_to(node: &Arc<Node>, lines: Vec<Line>) -> Result<Arc<Lines>> {
		let _ = node
			.get_aspect::<Spatial>()
			.unwrap()
			.bounding_box_calc
			.set(|node| {
				node.get_aspect::<Lines>()
					.ok()
					.map(|v| *v.bounds.lock())
					.unwrap_or_default()
			});

		let lines = LINES_REGISTRY.add(Lines {
			spatial: node.get_aspect::<Spatial>()?.clone(),
			data: Mutex::new(lines),
			gen_mesh: AtomicBool::new(true),
			entity: OnceLock::new(),
			bounds: Mutex::new(Aabb::default()),
		});
		node.add_aspect_raw(lines.clone());

		Ok(lines)
	}
}
impl LinesAspect for Lines {
	fn set_lines(node: Arc<Node>, _calling_client: Arc<Client>, lines: Vec<Line>) -> Result<()> {
		let lines_aspect = node.get_aspect::<Lines>()?;
		*lines_aspect.data.lock() = lines;
		lines_aspect.gen_mesh.store(true, Ordering::Relaxed);
		Ok(())
	}
}
impl Drop for Lines {
	fn drop(&mut self) {
		LINES_REGISTRY.remove(self);
	}
}

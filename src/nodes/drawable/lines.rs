use super::{Line, LinesAspect};
use crate::{
	core::{
		client::Client, color::ColorConvert, entity_handle::EntityHandle, error::Result,
		registry::Registry,
	},
	nodes::{
		Node,
		spatial::{Spatial, SpatialNode},
	},
};
use bevy::{
	asset::RenderAssetUsages,
	prelude::*,
	render::{
		mesh::{Indices, MeshAabb, PrimitiveTopology},
		primitives::Aabb,
	},
};
use bevy_sk::vr_materials::PbrMaterial;
use glam::Vec3;
use parking_lot::Mutex;
use std::sync::{
	Arc, OnceLock,
	atomic::{AtomicBool, Ordering},
};

pub struct LinesNodePlugin;

impl Plugin for LinesNodePlugin {
	fn build(&self, app: &mut App) {
		app.add_systems(Update, (build_line_mesh, update_visibillity).chain());
	}
}

fn update_visibillity(mut cmds: Commands) {
	for lines in LINES_REGISTRY.get_valid_contents().into_iter() {
		let Some(entity) = lines.entity.get().map(|e| **e) else {
			continue;
		};
		match lines.spatial.node().map(|n| n.enabled()).unwrap_or(false) {
			true => {
				cmds.entity(entity)
					.insert_recursive::<Children>(Visibility::Visible);
			}
			false => {
				cmds.entity(entity)
					.insert_recursive::<Children>(Visibility::Hidden);
			}
		}
	}
}

fn build_line_mesh(
	mut meshes: ResMut<Assets<Mesh>>,
	mut cmds: Commands,
	mut materials: ResMut<Assets<PbrMaterial>>,
) {
	for lines in LINES_REGISTRY
		.get_valid_contents()
		.into_iter()
		.filter(|l| l.gen_mesh.load(Ordering::Relaxed))
	{
		lines.gen_mesh.store(false, Ordering::Relaxed);
		let mut vertex_positions = Vec::<Vec3>::new();
		let mut vertex_normals = Vec::<Vec3>::new();
		let mut vertex_colors = Vec::<[f32; 4]>::new();
		let mut vertex_indecies = Vec::<u32>::new();
		let lines_data = lines.data.lock();
		if lines_data.is_empty() {
			*lines.bounds.lock() = Aabb::default();
			match lines.entity.get() {
				Some(e) => cmds.entity(**e),
				None => {
					let e = cmds.spawn((
						Name::new("LinesNode"),
						SpatialNode(Arc::downgrade(&lines.spatial)),
					));
					_ = lines.entity.set(e.id().into());
					e
				}
			}
			.remove::<Mesh3d>();
			continue;
		}

		let mut indecies_set = 0;
		for line in lines_data.iter() {
			let start_set = indecies_set;
			let optional_points = {
				let mut out = Vec::new();
				let mut last = line.cyclic.then(|| line.points.last()).flatten();
				let mut peekable = line.points.iter().peekable();
				while let Some(curr) = peekable.next() {
					let mut end = false;
					let next = match peekable.peek() {
						Some(v) => Some(*v),
						None => {
							end = true;
							line.cyclic.then(|| line.points.first()).flatten()
						}
					};

					out.push((last, curr, next, end));
					last = Some(curr);
				}
				out
			};
			for (last, curr, next, end) in optional_points {
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
					(None, None) => unreachable!(),
					(None, Some(next)) => next,
					(Some(last), None) => last,
					(Some(last), Some(next)) => last.lerp(next, 0.5),
				};
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
				.map(|v| (quat * v));
				let points = normals.map(|v| (v * curr.thickness) + Vec3::from(curr.point));
				vertex_normals.extend(normals);
				vertex_positions.extend(points);
				vertex_colors.extend([curr.color.to_bevy().to_srgba().to_f32_array(); 8]);
				if !end {
					vertex_indecies.extend(indecies(indecies_set));
				}
				indecies_set += 1;
			}
			if line.cyclic {
				vertex_indecies.extend(cyclic_indecies(start_set, indecies_set - 1));
			} else {
				vertex_indecies.extend(cap_indecies(start_set, false));
				vertex_indecies.extend(cap_indecies(indecies_set - 1, true));
			}
		}
		let mut mesh = Mesh::new(
			PrimitiveTopology::TriangleList,
			RenderAssetUsages::RENDER_WORLD,
		);
		mesh.insert_indices(Indices::U32(vertex_indecies));
		mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vertex_colors);
		mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vertex_normals);
		mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vertex_positions.clone());
		if let Some(aabb) = mesh.compute_aabb() {
			*lines.bounds.lock() = aabb;
		}

		match lines.entity.get() {
			Some(e) => cmds.entity(**e),
			None => {
				let e = cmds.spawn((
					Name::new("LinesNode"),
					SpatialNode(Arc::downgrade(&lines.spatial)),
					MeshMaterial3d(materials.add(PbrMaterial {
						color: Color::WHITE,
						roughness: 1.0,
						alpha_mode: AlphaMode::Opaque,
						..default()
					})),
				));
				_ = lines.entity.set(e.id().into());
				e
			}
		}
		.insert(Mesh3d(meshes.add(mesh)));
	}
}

const END_CAP_INDECIES: [u32; 18] = [0, 1, 7, 7, 1, 2, 7, 2, 6, 6, 2, 3, 6, 3, 5, 5, 3, 4];
fn cap_indecies(set: u32, flip: bool) -> [u32; END_CAP_INDECIES.len()] {
	let mut out = END_CAP_INDECIES.map(|v| v + (set * 8));
	if flip {
		out.reverse();
	}
	out
}

// const BASE: [u16; 6] = [0, 8, 1, 8, 9, 1];
const INDECIES: [u32; 48] = [
	0, 8, 1, 8, 9, 1, 1, 9, 2, 9, 10, 2, 2, 10, 3, 10, 11, 3, 3, 11, 4, 11, 12, 4, 4, 12, 5, 12,
	13, 5, 5, 13, 6, 13, 14, 6, 6, 14, 7, 14, 15, 7, 7, 15, 0, 15, 8, 0,
];
fn indecies(set: u32) -> [u32; INDECIES.len()] {
	INDECIES.map(|v| v + (set * 8))
}
fn cyclic_indecies(start_set: u32, end_set: u32) -> [u32; INDECIES.len()] {
	let mut out = INDECIES.map(|v| {
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

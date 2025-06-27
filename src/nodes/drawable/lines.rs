use super::{Line, LinesAspect};
use crate::{
	core::{client::Client, color::ColorConvert, error::Result, registry::Registry},
	nodes::{
		Node,
		spatial::{Spatial, SpatialNode},
	},
};
use bevy::{
	asset::RenderAssetUsages,
	prelude::*,
	render::mesh::{Indices, PrimitiveTopology},
};
use bevy_sk::vr_materials::PbrMaterial;
use glam::{FloatExt, Vec3};
use parking_lot::Mutex;
use std::{
	collections::VecDeque,
	sync::{
		Arc, OnceLock,
		atomic::{AtomicBool, Ordering},
	},
};
use stereokit_rust::{
	maths::Bounds, sk::MainThreadToken, system::LinePoint as SkLinePoint, util::Color128,
};

pub struct LinesNodePlugin;

impl Plugin for LinesNodePlugin {
	fn build(&self, app: &mut App) {
		app.add_systems(Update, build_line_mesh);
	}
}
const POINTS: [Vec3; 8] = [
	Vec3::X,
	Vec3::new(1., 0., 1.),
	Vec3::Z,
	Vec3::new(-1., 0., 1.),
	Vec3::NEG_X,
	Vec3::new(-1., 0., -1.),
	Vec3::NEG_Z,
	Vec3::new(1., 0., -1.),
];

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
			continue;
		}
	
		let mut idk = 0;
		for line in lines_data.iter() {
			let optional_points = {
				let mut out = Vec::new();
				let mut last = line.cyclic.then(|| line.points.last()).flatten();
				let mut peekable = line.points.iter().peekable();
				while let Some(curr) = peekable.next() {
					let next = match peekable.peek() {
						Some(v) => Some(*v),
						None => line.cyclic.then(|| line.points.first()).flatten(),
					};

					out.push((last, curr, next));
					last = Some(curr);
				}
				out
			};
			for (last, curr, next) in optional_points {
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
				idk += 1;
			}
		}
		let vertex_indecies = (0..idk - 1).flat_map(indecies).collect::<Vec<_>>();
		let mut mesh = Mesh::new(
			PrimitiveTopology::TriangleList,
			RenderAssetUsages::RENDER_WORLD,
		);
		mesh.insert_indices(Indices::U32(vertex_indecies));
		mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vertex_colors);
		mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vertex_normals);
		mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vertex_positions);

		match lines.entity.get() {
			Some(e) => cmds.entity(*e),
			None => cmds.spawn(SpatialNode(Arc::downgrade(&lines.spatial))),
		}
		.insert((
			Mesh3d(meshes.add(mesh)),
			MeshMaterial3d(materials.add(PbrMaterial {
				color: Color::WHITE,
				roughness: 1.0,
				alpha_mode: AlphaMode::Blend,
				..default()
			})),
		));
	}
}

// const BASE: [u16; 6] = [0, 8, 1, 8, 9, 1];
const INDECIES: [u32; 48] = [
	0, 8, 1, 8, 9, 1, 1, 9, 2, 9, 10, 2, 2, 10, 3, 10, 11, 3, 3, 11, 4, 11, 12, 4, 4, 12, 5, 12,
	13, 5, 5, 13, 6, 13, 14, 6, 6, 14, 7, 14, 15, 7, 7, 15, 0, 15, 8, 0,
];
fn indecies(base: u32) -> [u32; INDECIES.len()] {
	INDECIES.map(|v| v + (base * 8))
}
fn cyclic_indecies(base: u32) -> [u32; INDECIES.len()] {
	let mut out = INDECIES.map(|v| if v >= 8 { v + (base * 8) } else { v });
	out.reverse();
	out
}

static LINES_REGISTRY: Registry<Lines> = Registry::new();

pub struct Lines {
	spatial: Arc<Spatial>,
	data: Mutex<Vec<Line>>,
	gen_mesh: AtomicBool,
	entity: OnceLock<Entity>,
}
impl Lines {
	pub fn add_to(node: &Arc<Node>, lines: Vec<Line>) -> Result<Arc<Lines>> {
		let _ = node
			.get_aspect::<Spatial>()
			.unwrap()
			.bounding_box_calc
			.set(|node| {
				let mut bounds = Bounds::default();
				if let Ok(lines) = node.get_aspect::<Lines>() {
					for line in &*lines.data.lock() {
						for point in &line.points {
							bounds.grown_point(Vec3::from(point.point));
						}
					}
				}
				bounds
			});

		info!("line::add_to");
		let lines = LINES_REGISTRY.add(Lines {
			spatial: node.get_aspect::<Spatial>()?.clone(),
			data: Mutex::new(lines),
			gen_mesh: AtomicBool::new(true),
			entity: OnceLock::new(),
		});
		node.add_aspect_raw(lines.clone());

		Ok(lines)
	}

	fn draw(&self, token: &MainThreadToken) {
		let transform_mat = self.spatial.global_transform();
		let data = self.data.lock().clone();
		for line in &data {
			let mut points: VecDeque<SkLinePoint> = line
				.points
				.iter()
				.map(|p| SkLinePoint {
					pt: transform_mat.transform_point3(Vec3::from(p.point)).into(),
					thickness: p.thickness,
					color: Color128::new(p.color.c.r, p.color.c.g, p.color.c.b, p.color.a).into(),
				})
				.collect();
			if line.cyclic && !points.is_empty() {
				let first = line.points.first().unwrap();
				let last = line.points.last().unwrap();

				let color = Color128 {
					r: first.color.c.r.lerp(last.color.c.r, 0.5),
					g: first.color.c.g.lerp(last.color.c.g, 0.5),
					b: first.color.c.b.lerp(last.color.c.b, 0.5),
					a: first.color.a.lerp(last.color.a, 0.5),
				};
				let connect_point = SkLinePoint {
					pt: transform_mat
						.transform_point3(Vec3::from(first.point).lerp(Vec3::from(last.point), 0.5))
						.into(),
					thickness: (first.thickness + last.thickness) * 0.5,
					color: color.into(),
				};
				points.push_front(connect_point);
				points.push_back(connect_point);
			}
			stereokit_rust::system::Lines::add_list(token, points.make_contiguous());
		}
	}
}
impl LinesAspect for Lines {
	fn set_lines(node: Arc<Node>, _calling_client: Arc<Client>, lines: Vec<Line>) -> Result<()> {
		info!("set_lines");
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

pub fn draw_all(token: &MainThreadToken) {
	for lines in LINES_REGISTRY.get_valid_contents() {
		if let Some(node) = lines.spatial.node() {
			if node.enabled() {
				lines.draw(token);
			}
		}
	}
}

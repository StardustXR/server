use super::{Line, LinesAspect};
use crate::{
	bevy_plugin::{StardustExtract, TemporaryEntity, ViewLocation},
	core::{client::Client, registry::Registry},
	nodes::{spatial::Spatial, Node},
};
use bevy::{
	app::Plugin,
	asset::{Assets, RenderAssetUsages},
	color::{Color, ColorToComponents, Srgba},
	math::{bounding::Aabb3d, Isometry3d},
	pbr::{MeshMaterial3d, StandardMaterial},
	prelude::{
		AlphaMode, Commands, GlobalTransform, Mesh, Mesh3d, ResMut, Single, Transform, With,
	},
};
use color_eyre::eyre::Result;
use glam::{Vec3, Vec3A};
use parking_lot::Mutex;
use prisma::Lerp;
use std::{collections::VecDeque, sync::Arc};

static LINES_REGISTRY: Registry<Lines> = Registry::new();

pub struct Lines {
	space: Arc<Spatial>,
	data: Mutex<Vec<Line>>,
}
impl Lines {
	pub fn add_to(node: &Arc<Node>, lines: Vec<Line>) -> Result<Arc<Lines>> {
		*node
			.get_aspect::<Spatial>()
			.unwrap()
			.bounding_box_calc
			.lock() = {
			if let Ok(lines) = node.get_aspect::<Lines>() {
				Aabb3d::from_point_cloud(
					Isometry3d::IDENTITY,
					lines
						.data
						.lock()
						.iter()
						.flat_map(|line| line.points.iter())
						.map(|point| Vec3A::from(point.point)),
				)
			} else {
				Aabb3d::new(Vec3A::ZERO, Vec3A::ZERO)
			}
		};

		let lines = LINES_REGISTRY.add(Lines {
			space: node.get_aspect::<Spatial>()?.clone(),
			data: Mutex::new(lines),
		});
		node.add_aspect_raw(lines.clone());

		Ok(lines)
	}

	fn draw(&self, mesh: &mut Mesh, view: &GlobalTransform) -> Transform {
		let transform_mat = self.space.global_transform();
		let data = self.data.lock().clone();
		let global_to_view = view.compute_matrix().inverse();
		let local_to_view = transform_mat.inverse() * global_to_view;
		let view_to_local = local_to_view.inverse();
		for line in &data {
			let mut points: VecDeque<BevyLinePoint> = line
				.points
				.iter()
				.map(|p| BevyLinePoint {
					pt: transform_mat.transform_point3(Vec3::from(p.point)).into(),
					thickness: p.thickness,
					color: Srgba::new(p.color.c.r, p.color.c.g, p.color.c.b, p.color.a).into(),
				})
				.collect();
			if line.cyclic && !points.is_empty() {
				let first = line.points.first().unwrap();
				let last = line.points.last().unwrap();

				let color = Srgba {
					red: first.color.c.r.lerp(&last.color.c.r, 0.5),
					green: first.color.c.g.lerp(&last.color.c.g, 0.5),
					blue: first.color.c.b.lerp(&last.color.c.b, 0.5),
					alpha: first.color.a.lerp(&last.color.a, 0.5),
				};
				let connect_point = BevyLinePoint {
					pt: transform_mat
						.transform_point3(Vec3::from(first.point).lerp(Vec3::from(last.point), 0.5))
						.into(),
					thickness: (first.thickness + last.thickness) * 0.5,
					color: color.into(),
				};
				points.push_front(connect_point);
				points.push_back(connect_point);
			}
			let mut last_points: Option<(Vec3A, Vec3, Srgba)> = None;
			let mut vertecies: Vec<[f32; 3]> = Vec::new();
			let mut colors: Vec<[f32; 4]> = Vec::new();
			let mut normals: Vec<[f32; 3]> = Vec::new();
			for point in points.into_iter() {
				let pt_view = local_to_view.transform_point3a(point.pt.into());
				let point1_view = pt_view + (Vec3A::Y * (point.thickness / 2.0));
				let point2_view = pt_view + (Vec3A::NEG_Y * (point.thickness / 2.0));
				let point1 = view_to_local.transform_point3a(point1_view);
				let point2 = view_to_local.transform_point3a(point2_view);
				if let Some((last1, last2, last_color)) = last_points.take() {
					let normal = view_to_local.transform_vector3a(Vec3A::Z).to_array();
					for _ in 0..6 {
						normals.push(normal);
					}
					vertecies.push(last1.to_array());
					vertecies.push(point1.to_array());
					vertecies.push(last2.to_array());

					vertecies.push(last2.to_array());
					vertecies.push(point1.to_array());
					vertecies.push(point2.to_array());

					colors.push(last_color.to_f32_array());
					colors.push(point.color.to_f32_array());
					colors.push(last_color.to_f32_array());

					colors.push(last_color.to_f32_array());
					colors.push(point.color.to_f32_array());
					colors.push(point.color.to_f32_array());
				}
			}

			mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vertecies);
			mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
			mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
		}
		GlobalTransform::from(transform_mat).into()
	}
}
#[derive(Clone, Copy)]
struct BevyLinePoint {
	pt: Vec3,
	color: Srgba,
	thickness: f32,
}
impl Aspect for Lines {
	const NAME: &'static str = "Lines";
}
impl LinesAspect for Lines {
	fn set_lines(node: Arc<Node>, _calling_client: Arc<Client>, lines: Vec<Line>) -> Result<()> {
		let lines_aspect = node.get_aspect::<Lines>()?;
		*lines_aspect.data.lock() = lines;
		Ok(())
	}
}
impl Drop for Lines {
	fn drop(&mut self) {
		LINES_REGISTRY.remove(self);
	}
}

pub fn draw_all(
	mut meshes: ResMut<Assets<Mesh>>,
	mut materials: ResMut<Assets<StandardMaterial>>,
	mut cmds: Commands,
	hmd: Single<&GlobalTransform, With<ViewLocation>>,
) {
	let material = StandardMaterial {
		base_color: Color::WHITE,
		alpha_mode: AlphaMode::Blend,
		..Default::default()
	};
	let mat_handle = materials.add(material);
	for lines in LINES_REGISTRY.get_valid_contents() {
		if let Some(node) = lines.space.node() {
			if node.enabled() && !lines.data.lock().is_empty() {
				// Does this rebuild the mesh every frame? yes, is this problematic? probably,
				// would a shader work better? yes, do i care? not right now
				let mut mesh = Mesh::new(
					bevy::render::mesh::PrimitiveTopology::TriangleList,
					RenderAssetUsages::RENDER_WORLD,
				);
				let transform = lines.draw(&mut mesh, &hmd);
				let mesh_handle = meshes.add(mesh);
				cmds.spawn((
					Mesh3d(mesh_handle),
					MeshMaterial3d(mat_handle.clone()),
					TemporaryEntity,
					transform,
				));
			}
		}
	}
}
pub struct BevyLinesPlugin;
impl Plugin for BevyLinesPlugin {
	fn build(&self, app: &mut bevy::prelude::App) {
		app.add_systems(StardustExtract, draw_all);
	}
}

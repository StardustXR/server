use crate::{
	core::{client::Client, registry::Registry},
	nodes::{
		spatial::{find_spatial_parent, parse_transform, Spatial},
		Node,
	},
};
use color_eyre::eyre::{ensure, Result};
use glam::Vec3A;
use mint::Vector3;
use parking_lot::Mutex;
use prisma::{Flatten, Lerp, Rgba};
use serde::Deserialize;
use stardust_xr::{schemas::flex::deserialize, values::Transform};
use std::{collections::VecDeque, sync::Arc};
use stereokit::{lifecycle::StereoKitDraw, lines::LinePoint as SkLinePoint, values::Color128};

static LINES_REGISTRY: Registry<Lines> = Registry::new();

#[derive(Debug, Clone, Deserialize)]
struct LinePointRaw {
	point: Vector3<f32>,
	thickness: f32,
	color: [f32; 4],
}

#[derive(Debug, Clone)]
struct LineData {
	points: Vec<LinePointRaw>,
	cyclic: bool,
}

pub struct Lines {
	space: Arc<Spatial>,
	data: Mutex<LineData>,
}
impl Lines {
	fn add_to(node: &Arc<Node>, points: Vec<LinePointRaw>, cyclic: bool) -> Result<Arc<Lines>> {
		ensure!(
			node.model.get().is_none(),
			"Internal: Node already has lines attached!"
		);

		let lines = LINES_REGISTRY.add(Lines {
			space: node.get_aspect("Lines", "spatial", |n| &n.spatial)?.clone(),
			data: Mutex::new(LineData { points, cyclic }),
		});
		node.add_local_signal("set_points", Lines::set_points_flex);
		node.add_local_signal("set_cyclic", Lines::set_cyclic_flex);
		let _ = node.lines.set(lines.clone());

		Ok(lines)
	}

	fn draw(&self, draw_ctx: &StereoKitDraw) {
		let transform_mat = self.space.global_transform();
		let data = self.data.lock().clone();
		let mut points: VecDeque<SkLinePoint> = data
			.points
			.iter()
			.map(|p| SkLinePoint {
				point: transform_mat.transform_point3a(Vec3A::from(p.point)).into(),
				thickness: p.thickness,
				color: p.color.map(|c| (c * 255.0) as u8).into(),
			})
			.collect();
		if data.cyclic && !points.is_empty() {
			let first = data.points.first().unwrap();
			let last = data.points.last().unwrap();
			let connect_point = SkLinePoint {
				point: transform_mat
					.transform_point3a(Vec3A::from(first.point).lerp(Vec3A::from(last.point), 0.5))
					.into(),
				thickness: (first.thickness + last.thickness) * 0.5,
				color: Color128::from(
					Rgba::from_slice(&first.color).lerp(&Rgba::from_slice(&last.color), 0.5),
				)
				.into(),
			};
			points.push_front(connect_point.clone());
			points.push_back(connect_point);
		}
		stereokit::lines::line_add_listv(draw_ctx, points.make_contiguous());
	}

	pub fn set_points_flex(node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		let mut points: Vec<LinePointRaw> = deserialize(data)?;
		for p in &mut points {
			p.color[0] = p.color[0].powf(2.2);
			p.color[1] = p.color[1].powf(2.2);
			p.color[2] = p.color[2].powf(2.2);
		}
		node.lines.get().unwrap().data.lock().points = points;
		Ok(())
	}
	pub fn set_cyclic_flex(node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		node.lines.get().unwrap().data.lock().cyclic = deserialize(data)?;
		Ok(())
	}
}
impl Drop for Lines {
	fn drop(&mut self) {
		LINES_REGISTRY.remove(self);
	}
}

pub fn draw_all(draw_ctx: &StereoKitDraw) {
	for lines in LINES_REGISTRY.get_valid_contents() {
		lines.draw(draw_ctx);
	}
}

pub fn create_flex(_node: &Node, calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
	#[derive(Deserialize)]
	struct CreateTextInfo<'a> {
		name: &'a str,
		parent_path: &'a str,
		transform: Transform,
		points: Vec<LinePointRaw>,
		cyclic: bool,
	}
	let mut info: CreateTextInfo = deserialize(data)?;
	let node = Node::create(&calling_client, "/drawable/lines", info.name, true);
	let parent = find_spatial_parent(&calling_client, info.parent_path)?;
	let transform = parse_transform(info.transform, true, true, true);

	for p in &mut info.points {
		p.color[0] = p.color[0].powf(2.2);
		p.color[1] = p.color[1].powf(2.2);
		p.color[2] = p.color[2].powf(2.2);
	}

	let node = node.add_to_scenegraph();
	Spatial::add_to(&node, Some(parent), transform, false)?;
	Lines::add_to(&node, info.points, info.cyclic)?;
	Ok(())
}

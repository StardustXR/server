use crate::{
	core::{client::Client, registry::Registry},
	nodes::{
		spatial::{find_spatial_parent, parse_transform, Spatial, Transform},
		Message, Node,
	},
};
use color_eyre::eyre::{bail, ensure, Result};
use glam::Vec3A;
use mint::Vector3;
use parking_lot::Mutex;
use portable_atomic::{AtomicBool, Ordering};
use prisma::{Flatten, Lerp, Rgba};
use serde::Deserialize;
use stardust_xr::schemas::flex::deserialize;
use std::{collections::VecDeque, sync::Arc};
use stereokit::{bounds_grow_to_fit_pt, Bounds, Color128, LinePoint as SkLinePoint, StereoKitDraw};

use super::Drawable;

static LINES_REGISTRY: Registry<Lines> = Registry::new();

#[derive(Debug, Clone, Deserialize)]
struct LinePointRaw {
	point: Vector3<f32>,
	thickness: f32,
	color: [f32; 4],
}
#[derive(Debug, Clone, Deserialize)]
struct Line {
	points: Vec<LinePointRaw>,
	cyclic: bool,
}
impl Line {
	fn degamma(&mut self) {
		for p in &mut self.points {
			p.color[0] = p.color[0].powf(2.2);
			p.color[1] = p.color[1].powf(2.2);
			p.color[2] = p.color[2].powf(2.2);
		}
	}
}

pub struct Lines {
	enabled: Arc<AtomicBool>,
	space: Arc<Spatial>,
	data: Mutex<Vec<Line>>,
}
impl Lines {
	fn add_to(node: &Arc<Node>, lines: Vec<Line>) -> Result<Arc<Lines>> {
		ensure!(
			node.drawable.get().is_none(),
			"Internal: Node already has a drawable attached!"
		);

		let _ = node.spatial.get().unwrap().bounding_box_calc.set(|node| {
			let mut bounds = Bounds::default();
			let Some(Drawable::Lines(lines)) = node.drawable.get() else {
				return bounds;
			};
			for line in &*lines.data.lock() {
				for point in &line.points {
					bounds = bounds_grow_to_fit_pt(bounds, point.point);
				}
			}

			bounds
		});

		let lines = LINES_REGISTRY.add(Lines {
			enabled: node.enabled.clone(),
			space: node.get_aspect("Lines", "spatial", |n| &n.spatial)?.clone(),
			data: Mutex::new(lines),
		});
		node.add_local_signal("set_lines", Lines::set_lines_flex);
		let _ = node.drawable.set(Drawable::Lines(lines.clone()));

		Ok(lines)
	}

	fn draw(&self, draw_ctx: &impl StereoKitDraw) {
		let transform_mat = self.space.global_transform();
		let data = self.data.lock().clone();
		for line in &data {
			let mut points: VecDeque<SkLinePoint> = line
				.points
				.iter()
				.map(|p| SkLinePoint {
					pt: transform_mat.transform_point3a(Vec3A::from(p.point)).into(),
					thickness: p.thickness,
					color: p.color.map(|c| (c * 255.0) as u8).into(),
				})
				.collect();
			if line.cyclic && !points.is_empty() {
				let first = line.points.first().unwrap();
				let last = line.points.last().unwrap();
				let color =
					Rgba::from_slice(&first.color).lerp(&Rgba::from_slice(&last.color), 0.5);
				let connect_point = SkLinePoint {
					pt: transform_mat
						.transform_point3a(
							Vec3A::from(first.point).lerp(Vec3A::from(last.point), 0.5),
						)
						.into(),
					thickness: (first.thickness + last.thickness) * 0.5,
					color: Color128::from([
						color.red(),
						color.green(),
						color.blue(),
						color.alpha(),
					])
					.into(),
				};
				points.push_front(connect_point.clone());
				points.push_back(connect_point);
			}
			draw_ctx.line_add_listv(points.make_contiguous());
		}
	}

	pub fn set_lines_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		message: Message,
	) -> Result<()> {
		let Some(Drawable::Lines(lines)) = node.drawable.get() else {
			bail!("Not a drawable??")
		};

		let mut new_lines: Vec<Line> = deserialize(message.as_ref())?;
		for l in &mut new_lines {
			l.degamma();
		}
		*lines.data.lock() = new_lines;
		Ok(())
	}
}
impl Drop for Lines {
	fn drop(&mut self) {
		LINES_REGISTRY.remove(self);
	}
}

pub fn draw_all(draw_ctx: &impl StereoKitDraw) {
	for lines in LINES_REGISTRY.get_valid_contents() {
		if lines.enabled.load(Ordering::Relaxed) {
			lines.draw(draw_ctx);
		}
	}
}

pub fn create_flex(_node: &Node, calling_client: Arc<Client>, message: Message) -> Result<()> {
	#[derive(Deserialize)]
	struct CreateLinesInfo<'a> {
		name: &'a str,
		parent_path: &'a str,
		transform: Transform,
		lines: Vec<Line>,
	}
	let mut info: CreateLinesInfo = deserialize(message.as_ref())?;
	let node = Node::create(&calling_client, "/drawable/lines", info.name, true);
	let parent = find_spatial_parent(&calling_client, info.parent_path)?;
	let transform = parse_transform(info.transform, true, true, true);

	for l in &mut info.lines {
		l.degamma();
	}

	let node = node.add_to_scenegraph()?;
	Spatial::add_to(&node, Some(parent), transform, false)?;
	Lines::add_to(&node, info.lines)?;
	Ok(())
}

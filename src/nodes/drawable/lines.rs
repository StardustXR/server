use super::{Line, LinesAspect};
use crate::{
	core::{client::Client, error::Result, registry::Registry},
	nodes::{spatial::Spatial, Node},
};
use glam::Vec3;
use parking_lot::Mutex;
use prisma::Lerp;
use std::{collections::VecDeque, sync::Arc};
use stereokit_rust::{
	maths::Bounds, sk::MainThreadToken, system::LinePoint as SkLinePoint, util::Color128,
};

static LINES_REGISTRY: Registry<Lines> = Registry::new();

pub struct Lines {
	space: Arc<Spatial>,
	data: Mutex<Vec<Line>>,
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

		let lines = LINES_REGISTRY.add(Lines {
			space: node.get_aspect::<Spatial>()?.clone(),
			data: Mutex::new(lines),
		});
		node.add_aspect_raw(lines.clone());

		Ok(lines)
	}

	fn draw(&self, token: &MainThreadToken) {
		let transform_mat = self.space.global_transform();
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
					r: first.color.c.r.lerp(&last.color.c.r, 0.5),
					g: first.color.c.g.lerp(&last.color.c.g, 0.5),
					b: first.color.c.b.lerp(&last.color.c.b, 0.5),
					a: first.color.a.lerp(&last.color.a, 0.5),
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

pub fn draw_all(token: &MainThreadToken) {
	for lines in LINES_REGISTRY.get_valid_contents() {
		if let Some(node) = lines.space.node() {
			if node.enabled() {
				lines.draw(token);
			}
		}
	}
}

use super::{Line, LinesAspect};
use crate::{
	core::{client::Client, registry::Registry},
	nodes::{spatial::Spatial, Aspect, Node},
};
use color_eyre::eyre::Result;
use glam::Vec3A;
use parking_lot::Mutex;
use portable_atomic::{AtomicBool, Ordering};
use prisma::Lerp;
use std::{collections::VecDeque, sync::Arc};
use stereokit::{bounds_grow_to_fit_pt, Bounds, Color128, LinePoint as SkLinePoint, StereoKitDraw};

static LINES_REGISTRY: Registry<Lines> = Registry::new();

pub struct Lines {
	enabled: Arc<AtomicBool>,
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
							bounds = bounds_grow_to_fit_pt(bounds, point.point);
						}
					}
				}
				bounds
			});

		let lines = LINES_REGISTRY.add(Lines {
			enabled: node.enabled.clone(),
			space: node.get_aspect::<Spatial>()?.clone(),
			data: Mutex::new(lines),
		});
		<Lines as LinesAspect>::add_node_members(node);
		node.add_aspect_raw(lines.clone());

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
					color: stereokit::sys::color128::from([
						p.color.c.r,
						p.color.c.g,
						p.color.c.b,
						p.color.a,
					])
					.into(),
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
						.transform_point3a(
							Vec3A::from(first.point).lerp(Vec3A::from(last.point), 0.5),
						)
						.into(),
					thickness: (first.thickness + last.thickness) * 0.5,
					color: color.into(),
				};
				points.push_front(connect_point.clone());
				points.push_back(connect_point);
			}
			draw_ctx.line_add_listv(points.make_contiguous());
		}
	}
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

pub fn draw_all(draw_ctx: &impl StereoKitDraw) {
	for lines in LINES_REGISTRY.get_valid_contents() {
		if lines.enabled.load(Ordering::Relaxed) {
			lines.draw(draw_ctx);
		}
	}
}

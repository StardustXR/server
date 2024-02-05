use super::{Field, FieldTrait, Node, TorusFieldAspect};
use crate::core::client::Client;
use crate::nodes::fields::FieldAspect;
use crate::nodes::spatial::Spatial;
use color_eyre::eyre::Result;
use glam::{swizzles::*, vec2, Vec3A};
use portable_atomic::AtomicF32;

use std::sync::atomic::Ordering;
use std::sync::Arc;

pub struct TorusField {
	space: Arc<Spatial>,
	radius_a: AtomicF32,
	radius_b: AtomicF32,
}

impl TorusField {
	pub fn add_to(node: &Arc<Node>, radius_a: f32, radius_b: f32) {
		let torus_field = TorusField {
			space: node.get_aspect::<Spatial>().unwrap().clone(),
			radius_a: AtomicF32::new(radius_a.abs()),
			radius_b: AtomicF32::new(radius_b.abs()),
		};
		<TorusField as FieldAspect>::add_node_members(node);
		<TorusField as TorusFieldAspect>::add_node_members(node);
		node.add_aspect(Field::Torus(torus_field));
	}

	pub fn set_size(&self, radius_a: f32, radius_b: f32) {
		self.radius_a.store(radius_a.abs(), Ordering::Relaxed);
		self.radius_b.store(radius_b.abs(), Ordering::Relaxed);
	}
}
impl FieldTrait for TorusField {
	fn local_distance(&self, p: Vec3A) -> f32 {
		let radius_a = self.radius_a.load(Ordering::Relaxed);
		let radius_b = self.radius_b.load(Ordering::Relaxed);
		let q = vec2(p.xz().length() - radius_a, p.y);
		q.length() - radius_b
	}
	fn spatial_ref(&self) -> &Spatial {
		self.space.as_ref()
	}
}
impl TorusFieldAspect for TorusField {
	fn set_size(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		radius_a: f32,
		radius_b: f32,
	) -> Result<()> {
		let this_field = node.get_aspect::<Field>()?;
		let Field::Torus(this_field) = &*this_field else {
			return Ok(());
		};
		this_field.set_size(radius_a, radius_b);
		Ok(())
	}
}

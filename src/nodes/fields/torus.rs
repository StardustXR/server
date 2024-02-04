use super::{get_field, Field, FieldTrait, Node, TorusFieldAspect};
use crate::core::client::Client;
use crate::nodes::fields::FieldAspect;
use crate::nodes::spatial::Spatial;
use crate::nodes::Message;
use color_eyre::eyre::{ensure, Result};
use glam::{swizzles::*, vec2, Vec3A};
use portable_atomic::AtomicF32;
use stardust_xr::schemas::flex::deserialize;

use std::sync::atomic::Ordering;
use std::sync::Arc;

pub struct TorusField {
	space: Arc<Spatial>,
	radius_a: AtomicF32,
	radius_b: AtomicF32,
}

impl TorusField {
	pub fn add_to(node: &Arc<Node>, radius_a: f32, radius_b: f32) -> Result<()> {
		ensure!(
			node.spatial.get().is_some(),
			"Internal: Node does not have a spatial attached!"
		);
		ensure!(
			node.field.get().is_none(),
			"Internal: Node already has a field attached!"
		);
		let torus_field = TorusField {
			space: node.spatial.get().unwrap().clone(),
			radius_a: AtomicF32::new(radius_a.abs()),
			radius_b: AtomicF32::new(radius_b.abs()),
		};
		<TorusField as FieldAspect>::add_node_members(node);
		<TorusField as TorusFieldAspect>::add_node_members(node);
		node.add_local_signal("set_size", TorusField::set_size_flex);
		let _ = node.field.set(Arc::new(Field::Torus(torus_field)));
		Ok(())
	}

	pub fn set_size(&self, radius_a: f32, radius_b: f32) {
		self.radius_a.store(radius_a.abs(), Ordering::Relaxed);
		self.radius_b.store(radius_b.abs(), Ordering::Relaxed);
	}

	pub fn set_size_flex(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		message: Message,
	) -> Result<()> {
		let Field::Torus(torus_field) = node.field.get().unwrap().as_ref() else {
			return Ok(());
		};
		let (radius_a, radius_b) = deserialize(message.as_ref())?;
		torus_field.set_size(radius_a, radius_b);

		Ok(())
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
		let Field::Torus(this_field) = &*get_field(&node)? else {
			return Ok(());
		};
		this_field.set_size(radius_a, radius_b);
		Ok(())
	}
}

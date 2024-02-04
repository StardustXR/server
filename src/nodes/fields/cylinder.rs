use super::{get_field, CylinderFieldAspect, Field, FieldTrait, Node};
use crate::core::client::Client;
use crate::nodes::fields::FieldAspect;
use crate::nodes::spatial::Spatial;
use color_eyre::eyre::{ensure, Result};
use glam::{swizzles::*, vec2, Vec3A};
use portable_atomic::AtomicF32;

use std::sync::atomic::Ordering;
use std::sync::Arc;

pub struct CylinderField {
	space: Arc<Spatial>,
	length: AtomicF32,
	radius: AtomicF32,
}

impl CylinderField {
	pub fn add_to(node: &Arc<Node>, length: f32, radius: f32) -> Result<()> {
		ensure!(
			node.spatial.get().is_some(),
			"Internal: Node does not have a spatial attached!"
		);
		ensure!(
			node.field.get().is_none(),
			"Internal: Node already has a field attached!"
		);
		let cylinder_field = CylinderField {
			space: node.spatial.get().unwrap().clone(),
			length: AtomicF32::new(length.abs()),
			radius: AtomicF32::new(radius.abs()),
		};
		<CylinderField as FieldAspect>::add_node_members(node);
		<CylinderField as CylinderFieldAspect>::add_node_members(node);
		let _ = node.field.set(Arc::new(Field::Cylinder(cylinder_field)));
		Ok(())
	}

	pub fn set_size(&self, length: f32, radius: f32) {
		self.length.store(length.abs(), Ordering::Relaxed);
		self.radius.store(radius.abs(), Ordering::Relaxed);
	}
}
impl FieldTrait for CylinderField {
	fn local_distance(&self, p: Vec3A) -> f32 {
		let radius = self.radius.load(Ordering::Relaxed);
		let length = self.length.load(Ordering::Relaxed);
		let d = vec2(p.xy().length().abs() - radius, p.z.abs() - (length * 0.5));

		d.x.max(d.y).min(0.0) + d.max(vec2(0.0, 0.0)).length()
	}
	fn spatial_ref(&self) -> &Spatial {
		self.space.as_ref()
	}
}
impl CylinderFieldAspect for CylinderField {
	fn set_size(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		length: f32,
		radius: f32,
	) -> Result<()> {
		let Field::Cylinder(this_field) = &*get_field(&node)? else {
			return Ok(());
		};
		this_field.set_size(length, radius);
		Ok(())
	}
}

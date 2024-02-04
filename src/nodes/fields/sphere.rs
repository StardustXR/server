use super::{get_field, Field, FieldTrait, Node, SphereFieldAspect};
use crate::core::client::Client;
use crate::nodes::fields::FieldAspect;
use crate::nodes::spatial::Spatial;
use color_eyre::eyre::{ensure, Result};
use glam::Vec3A;
use portable_atomic::AtomicF32;
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub struct SphereField {
	space: Arc<Spatial>,
	radius: AtomicF32,
}

impl SphereField {
	pub fn add_to(node: &Arc<Node>, radius: f32) -> Result<()> {
		ensure!(
			node.spatial.get().is_some(),
			"Internal: Node does not have a spatial attached!"
		);
		ensure!(
			node.field.get().is_none(),
			"Internal: Node already has a field attached!"
		);
		let sphere_field = SphereField {
			space: node.spatial.get().unwrap().clone(),
			radius: AtomicF32::new(radius),
		};
		<SphereField as FieldAspect>::add_node_members(node);
		<SphereField as SphereFieldAspect>::add_node_members(node);
		let _ = node.field.set(Arc::new(Field::Sphere(sphere_field)));
		Ok(())
	}

	pub fn set_radius(&self, radius: f32) {
		self.radius.store(radius, Ordering::Relaxed);
	}
}

impl FieldTrait for SphereField {
	fn local_distance(&self, p: Vec3A) -> f32 {
		p.length() - self.radius.load(Ordering::Relaxed)
	}
	fn local_normal(&self, p: Vec3A, _r: f32) -> Vec3A {
		-p.normalize()
	}
	fn local_closest_point(&self, p: Vec3A, _r: f32) -> Vec3A {
		p.normalize() * self.radius.load(Ordering::Relaxed)
	}
	fn spatial_ref(&self) -> &Spatial {
		self.space.as_ref()
	}
}
impl SphereFieldAspect for SphereField {
	fn set_radius(node: Arc<Node>, _calling_client: Arc<Client>, radius: f32) -> Result<()> {
		let Field::Sphere(this_field) = &*get_field(&node)? else {
			return Ok(());
		};
		this_field.set_radius(radius);
		Ok(())
	}
}

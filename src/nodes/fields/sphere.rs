use super::{Field, FieldTrait, Node};
use crate::core::client::Client;
use crate::nodes::spatial::{get_spatial_parent_flex, Spatial};
use anyhow::{anyhow, ensure, Result};
use glam::{Mat4, Vec3A};
use libstardustxr::flex_to_vec3;
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
		sphere_field.add_field_methods(node);
		node.add_local_signal("setRadius", SphereField::set_radius_flex);
		let _ = node.field.set(Arc::new(Field::Sphere(sphere_field)));
		Ok(())
	}

	pub fn set_radius(&self, radius: f32) {
		self.radius.store(radius, Ordering::Relaxed);
	}

	pub fn set_radius_flex(node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		let root = flexbuffers::Reader::get_root(data)?;
		if let Field::Sphere(sphere_field) = node.field.get().unwrap().as_ref() {
			sphere_field.set_radius(root.as_f32());
		}
		Ok(())
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

pub fn create_sphere_field_flex(
	_node: &Node,
	calling_client: Arc<Client>,
	data: &[u8],
) -> Result<()> {
	let flex_vec = flexbuffers::Reader::get_root(data)?.get_vector()?;
	let node = Node::create(&calling_client, "/field", flex_vec.idx(0).get_str()?, true);
	let parent = get_spatial_parent_flex(&calling_client, flex_vec.idx(1).get_str()?)?;
	let transform = Mat4::from_translation(
		flex_to_vec3!(flex_vec.idx(2))
			.ok_or_else(|| anyhow!("Position not found"))?
			.into(),
	);
	let node = node.add_to_scenegraph();
	Spatial::add_to(&node, Some(parent), transform)?;
	SphereField::add_to(&node, flex_vec.idx(3).as_f32())?;
	Ok(())
}

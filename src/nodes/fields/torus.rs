use super::{Field, FieldTrait, Node};
use crate::core::client::Client;
use crate::nodes::spatial::{find_spatial_parent, parse_transform, Spatial};
use color_eyre::eyre::{ensure, Result};
use glam::{swizzles::*, vec2, Vec3A};
use portable_atomic::AtomicF32;
use serde::Deserialize;
use stardust_xr::schemas::flex::deserialize;
use stardust_xr::values::Transform;

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
		torus_field.add_field_methods(node);
		node.add_local_signal("set_size", TorusField::set_size_flex);
		let _ = node.field.set(Arc::new(Field::Torus(torus_field)));
		Ok(())
	}

	pub fn set_size(&self, radius_a: f32, radius_b: f32) {
		self.radius_a.store(radius_a.abs(), Ordering::Relaxed);
		self.radius_b.store(radius_b.abs(), Ordering::Relaxed);
	}

	pub fn set_size_flex(node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		let Field::Torus(torus_field) = node.field.get().unwrap().as_ref() else { return Ok(()) };
		let (radius_a, radius_b) = deserialize(data)?;
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

pub fn create_torus_field_flex(
	_node: &Node,
	calling_client: Arc<Client>,
	data: &[u8],
) -> Result<()> {
	#[derive(Deserialize)]
	struct CreateFieldInfo<'a> {
		name: &'a str,
		parent_path: &'a str,
		transform: Transform,
		radius_a: f32,
		radius_b: f32,
	}
	let info: CreateFieldInfo = deserialize(data)?;
	let node = Node::create(&calling_client, "/field", info.name, true);
	let parent = find_spatial_parent(&calling_client, info.parent_path)?;
	let transform = parse_transform(info.transform, true, true, false)?;
	let node = node.add_to_scenegraph();
	Spatial::add_to(&node, Some(parent), transform, false)?;
	TorusField::add_to(&node, info.radius_a, info.radius_b)?;
	Ok(())
}

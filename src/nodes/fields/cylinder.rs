use super::{Field, FieldTrait, Node};
use crate::core::client::Client;
use crate::nodes::spatial::{get_spatial_parent_flex, parse_transform, Spatial};
use anyhow::{ensure, Result};
use glam::{swizzles::*, vec2, Vec3A};
use portable_atomic::AtomicF32;
use stardust_xr::values::parse_f32;

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
		cylinder_field.add_field_methods(node);
		node.add_local_signal("setSize", CylinderField::set_size_flex);
		let _ = node.field.set(Arc::new(Field::Cylinder(cylinder_field)));
		Ok(())
	}

	pub fn set_size(&self, length: f32, radius: f32) {
		self.length.store(length.abs(), Ordering::Relaxed);
		self.radius.store(radius.abs(), Ordering::Relaxed);
	}

	pub fn set_size_flex(node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		let flex_vec = flexbuffers::Reader::get_root(data)?.get_vector()?;
		let length =
			parse_f32(flex_vec.index(0)?).ok_or_else(|| anyhow::anyhow!("Invalid length"))?;
		let radius =
			parse_f32(flex_vec.index(1)?).ok_or_else(|| anyhow::anyhow!("Invalid radius"))?;
		if let Field::Cylinder(cylinder_field) = node.field.get().unwrap().as_ref() {
			cylinder_field.set_size(length, radius);
		}
		Ok(())
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

pub fn create_cylinder_field_flex(
	_node: &Node,
	calling_client: Arc<Client>,
	data: &[u8],
) -> Result<()> {
	let flex_vec = flexbuffers::Reader::get_root(data)?.get_vector()?;
	let node = Node::create(
		&calling_client,
		"/field",
		flex_vec.index(0)?.get_str()?,
		true,
	);
	let parent = get_spatial_parent_flex(&calling_client, flex_vec.idx(1).get_str()?)?;
	let transform = parse_transform(flex_vec.index(2)?, true, true, false)?;
	let length = parse_f32(flex_vec.index(3)?).ok_or_else(|| anyhow::anyhow!("Invalid length"))?;
	let radius = parse_f32(flex_vec.index(4)?).ok_or_else(|| anyhow::anyhow!("Invalid radius"))?;
	let node = node.add_to_scenegraph();
	Spatial::add_to(&node, Some(parent), transform)?;
	CylinderField::add_to(&node, dbg!(length), dbg!(radius))?;
	Ok(())
}

use super::{Field, FieldTrait, Node};
use crate::core::client::Client;
use crate::nodes::spatial::{get_spatial_parent_flex, parse_transform, Spatial};
use anyhow::{anyhow, ensure, Result};
use glam::{vec3, vec3a, Vec3, Vec3A};
use parking_lot::Mutex;
use stardust_xr::values::parse_vec3;
use std::sync::Arc;

pub struct BoxField {
	space: Arc<Spatial>,
	size: Mutex<Vec3>,
}

impl BoxField {
	pub fn add_to(node: &Arc<Node>, size: Vec3) -> Result<()> {
		ensure!(
			node.spatial.get().is_some(),
			"Internal: Node does not have a spatial attached!"
		);
		ensure!(
			node.field.get().is_none(),
			"Internal: Node already has a field attached!"
		);
		let box_field = BoxField {
			space: node.spatial.get().unwrap().clone(),
			size: Mutex::new(size),
		};
		box_field.add_field_methods(node);
		node.add_local_signal("setSize", BoxField::set_size_flex);
		let _ = node.field.set(Arc::new(Field::Box(box_field)));
		Ok(())
	}

	pub fn set_size(&self, size: Vec3) {
		*self.size.lock() = size;
	}

	pub fn set_size_flex(node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		let root = flexbuffers::Reader::get_root(data)?;
		let size = parse_vec3(root).ok_or_else(|| anyhow!("Size is invalid"))?;
		if let Field::Box(box_field) = node.field.get().unwrap().as_ref() {
			box_field.set_size(size.into());
		}
		Ok(())
	}
}

impl FieldTrait for BoxField {
	fn local_distance(&self, p: Vec3A) -> f32 {
		let size = self.size.lock();
		let q = vec3(
			p.x.abs() - (size.x * 0.5_f32),
			p.y.abs() - (size.y * 0.5_f32),
			p.z.abs() - (size.z * 0.5_f32),
		);
		let v = vec3a(q.x.max(0_f32), q.y.max(0_f32), q.z.max(0_f32));
		v.length() + q.x.max(q.y.max(q.z)).min(0_f32)
	}
	fn spatial_ref(&self) -> &Spatial {
		self.space.as_ref()
	}
}

pub fn create_box_field_flex(_node: &Node, calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
	let flex_vec = flexbuffers::Reader::get_root(data)?.get_vector()?;
	let node = Node::create(
		&calling_client,
		"/field",
		flex_vec.index(0)?.get_str()?,
		true,
	);
	let parent = get_spatial_parent_flex(&calling_client, flex_vec.index(1)?.get_str()?)?;
	let transform = parse_transform(flex_vec.index(2)?, true, true, false)?;
	let size = parse_vec3(flex_vec.idx(4)).ok_or_else(|| anyhow!("Size invalid"))?;
	let node = node.add_to_scenegraph();
	Spatial::add_to(&node, Some(parent), transform)?;
	BoxField::add_to(&node, size.into())?;
	Ok(())
}

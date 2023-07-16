use super::{Field, FieldTrait, Node};
use crate::core::client::Client;
use crate::nodes::spatial::{find_spatial_parent, parse_transform, Spatial};
use color_eyre::eyre::{ensure, Result};
use glam::{vec3, vec3a, Vec3, Vec3A};
use mint::Vector3;
use parking_lot::Mutex;
use serde::Deserialize;
use stardust_xr::schemas::flex::deserialize;
use stardust_xr::values::Transform;
use std::sync::Arc;

pub struct BoxField {
	space: Arc<Spatial>,
	size: Mutex<Vec3>,
}

impl BoxField {
	pub fn add_to(node: &Arc<Node>, size: Vector3<f32>) -> Result<Arc<Field>> {
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
			size: Mutex::new(size.into()),
		};
		box_field.add_field_methods(node);
		node.add_local_signal("set_size", BoxField::set_size_flex);
		let field = Arc::new(Field::Box(box_field));
		let _ = node.field.set(field.clone());
		Ok(field)
	}

	pub fn set_size(&self, size: Vector3<f32>) {
		*self.size.lock() = size.into();
	}

	pub fn set_size_flex(node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		let Field::Box(box_field) = node.field.get().unwrap().as_ref() else { return Ok(()) };
		box_field.set_size(deserialize(data)?);

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
	#[derive(Deserialize)]
	struct CreateFieldInfo<'a> {
		name: &'a str,
		parent_path: &'a str,
		transform: Transform,
		size: Vector3<f32>,
	}
	let info: CreateFieldInfo = deserialize(data)?;
	let node = Node::create(&calling_client, "/field", info.name, true);
	let parent = find_spatial_parent(&calling_client, info.parent_path)?;
	let transform = parse_transform(info.transform, true, true, false);
	let node = node.add_to_scenegraph()?;
	Spatial::add_to(&node, Some(parent), transform, false)?;
	BoxField::add_to(&node, info.size)?;
	Ok(())
}

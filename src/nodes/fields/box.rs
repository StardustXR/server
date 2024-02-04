use super::{get_field, BoxFieldAspect, Field, FieldTrait, Node};
use crate::core::client::Client;
use crate::nodes::fields::FieldAspect;
use crate::nodes::spatial::Spatial;
use color_eyre::eyre::{ensure, Result};
use glam::{vec3, vec3a, Vec3, Vec3A};
use mint::Vector3;
use parking_lot::Mutex;
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
		<BoxField as FieldAspect>::add_node_members(node);
		<BoxField as BoxFieldAspect>::add_node_members(node);
		let field = Arc::new(Field::Box(box_field));
		let _ = node.field.set(field.clone());
		Ok(field)
	}

	pub fn set_size(&self, size: Vector3<f32>) {
		*self.size.lock() = size.into();
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
impl BoxFieldAspect for BoxField {
	fn set_size(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		size: mint::Vector3<f32>,
	) -> Result<()> {
		let Field::Box(this_field) = &*get_field(&node)? else {
			return Ok(());
		};
		this_field.set_size(size.into());
		Ok(())
	}
}

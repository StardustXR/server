use super::{BoxFieldAspect, FieldTrait, Node};
use crate::nodes::fields::FieldAspect;
use crate::nodes::spatial::Spatial;
use crate::{core::client::Client, nodes::fields::Field};
use color_eyre::eyre::Result;
use glam::{vec3, vec3a, Vec3, Vec3A};
use mint::Vector3;
use parking_lot::Mutex;
use std::sync::Arc;

pub struct BoxField {
	space: Arc<Spatial>,
	size: Mutex<Vec3>,
}

impl BoxField {
	pub fn add_to(node: &Arc<Node>, size: Vector3<f32>) {
		let box_field = BoxField {
			space: node.get_aspect::<Spatial>().unwrap().clone(),
			size: Mutex::new(size.into()),
		};
		<BoxField as FieldAspect>::add_node_members(node);
		<BoxField as BoxFieldAspect>::add_node_members(node);
		node.add_aspect(Field::Box(box_field));
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
		let this_field = node.get_aspect::<Field>()?;
		let Field::Box(this_field) = &*this_field else {
			return Ok(());
		};
		this_field.set_size(size);
		Ok(())
	}
}

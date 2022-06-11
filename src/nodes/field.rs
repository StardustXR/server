use super::core::Node;
use super::spatial::Spatial;
use anyhow::{anyhow, ensure, Result};
use glam::{vec2, vec3a, Vec3, Vec3A};
use libstardustxr::flex_to_vec3;
use libstardustxr::fusion::flex::FlexBuffable;
use rccell::RcCell;
use std::boxed::Box;
use std::rc::Rc;

pub trait Field {
	fn local_distance(&self, p: Vec3A) -> f32;
	fn local_normal(&self, p: Vec3A, r: f32) -> Vec3A {
		let d = self.local_distance(p);
		let e = vec2(r, 0_f32);

		let n = vec3a(d, d, d)
			- vec3a(
				self.local_distance(vec3a(e.x, e.y, e.y)),
				self.local_distance(vec3a(e.y, e.x, e.y)),
				self.local_distance(vec3a(e.y, e.y, e.x)),
			);

		n.normalize()
	}
	fn local_closest_point(&self, p: Vec3A, r: f32) -> Vec3A {
		p - (self.local_normal(p, r) * self.local_distance(p))
	}

	fn distance(&self, local_space: &Spatial, reference_space: &Spatial, p: Vec3A) -> f32 {
		let reference_to_local_space =
			Spatial::space_to_space_matrix(Some(reference_space), Some(local_space));
		let local_p = reference_to_local_space.transform_point3a(p);
		self.local_distance(local_p)
	}
	fn normal(&self, local_space: &Spatial, reference_space: &Spatial, p: Vec3A, r: f32) -> Vec3A {
		let reference_to_local_space =
			Spatial::space_to_space_matrix(Some(reference_space), Some(local_space));
		let local_p = reference_to_local_space.transform_point3a(p);
		reference_to_local_space
			.inverse()
			.transform_vector3a(self.local_normal(local_p, r))
	}
	fn closest_point(
		&self,
		local_space: &Spatial,
		reference_space: &Spatial,
		p: Vec3A,
		r: f32,
	) -> Vec3A {
		let reference_to_local_space =
			Spatial::space_to_space_matrix(Some(reference_space), Some(local_space));
		let local_p = reference_to_local_space.transform_point3a(p);
		reference_to_local_space
			.inverse()
			.transform_point3a(self.local_closest_point(local_p, r))
	}

	fn add_field_methods(&self, node: &RcCell<Node>) {
		node.borrow_mut()
			.add_local_method("distance", |node, calling_client, data| {
				let root = flexbuffers::Reader::get_root(data)?;
				let flex_vec = root.get_vector()?;
				let reference_space_path = flex_vec.idx(0).as_str();
				let reference_space = calling_client
					.get_scenegraph()
					.get_node(reference_space_path)
					.ok_or_else(|| anyhow!("Reference space node does not exist"))?
					.borrow()
					.spatial
					.as_ref()
					.ok_or_else(|| anyhow!("Reference space node does not have a spatial"))?
					.clone();
				let point =
					flex_to_vec3!(flex_vec.idx(1)).ok_or_else(|| anyhow!("Point is invalid"))?;

				let field = node
					.field
					.as_ref()
					.ok_or_else(|| anyhow!("Node does not have a field!"))?;
				let spatial = field.spatial_ref();
				let distance = field.distance(spatial, reference_space.as_ref(), point.into());
				Ok(FlexBuffable::from(distance).build_singleton())
			});
		node.borrow_mut()
			.add_local_method("normal", |node, calling_client, data| {
				let root = flexbuffers::Reader::get_root(data)?;
				let flex_vec = root.get_vector()?;
				let reference_space_path = flex_vec.idx(0).as_str();
				let reference_space = calling_client
					.get_scenegraph()
					.get_node(reference_space_path)
					.ok_or_else(|| anyhow!("Reference space node does not exist"))?
					.borrow()
					.spatial
					.as_ref()
					.ok_or_else(|| anyhow!("Reference space node does not have a spatial"))?
					.clone();
				let point =
					flex_to_vec3!(flex_vec.idx(1)).ok_or_else(|| anyhow!("Point is invalid"))?;

				let field = node
					.field
					.as_ref()
					.ok_or_else(|| anyhow!("Node does not have a field!"))?;
				let spatial = field.spatial_ref();
				let normal =
					field.normal(spatial, reference_space.as_ref(), point.into(), 0.001_f32);
				Ok(FlexBuffable::from(mint::Vector3::from(normal)).build_singleton())
			});
		node.borrow_mut()
			.add_local_method("closest_point", |node, calling_client, data| {
				let root = flexbuffers::Reader::get_root(data)?;
				let flex_vec = root.get_vector()?;
				let reference_space_path = flex_vec.idx(0).as_str();
				let reference_space = calling_client
					.get_scenegraph()
					.get_node(reference_space_path)
					.ok_or_else(|| anyhow!("Reference space node does not exist"))?
					.borrow()
					.spatial
					.as_ref()
					.ok_or_else(|| anyhow!("Reference space node does not have a spatial"))?
					.clone();
				let point =
					flex_to_vec3!(flex_vec.idx(1)).ok_or_else(|| anyhow!("Point is invalid"))?;

				let field = node
					.field
					.as_ref()
					.ok_or_else(|| anyhow!("Node does not have a field!"))?;
				let spatial = field.spatial_ref();
				let closest_point =
					field.closest_point(spatial, reference_space.as_ref(), point.into(), 0.001_f32);
				Ok(FlexBuffable::from(mint::Vector3::from(closest_point)).build_singleton())
			});
	}

	fn spatial_ref(&self) -> &Spatial;
}

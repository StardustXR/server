use super::core::Node;
use super::spatial::Spatial;
use crate::core::client::Client;
use anyhow::{anyhow, ensure, Result};
use glam::{swizzles::*, vec2, vec3, vec3a, Mat4, Vec3, Vec3A};
use libstardustxr::fusion::flex::FlexBuffable;
use libstardustxr::{flex_to_quat, flex_to_vec3};
use rccell::RcCell;
use std::cell::Cell;
use std::ops::Deref;
use std::rc::Rc;

pub trait FieldTrait {
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

	fn distance(&self, reference_space: &Spatial, p: Vec3A) -> f32 {
		let reference_to_local_space =
			Spatial::space_to_space_matrix(Some(reference_space), Some(self.spatial_ref()));
		let local_p = reference_to_local_space.transform_point3a(p);
		self.local_distance(local_p)
	}
	fn normal(&self, reference_space: &Spatial, p: Vec3A, r: f32) -> Vec3A {
		let reference_to_local_space =
			Spatial::space_to_space_matrix(Some(reference_space), Some(self.spatial_ref()));
		let local_p = reference_to_local_space.transform_point3a(p);
		reference_to_local_space
			.inverse()
			.transform_vector3a(self.local_normal(local_p, r))
	}
	fn closest_point(&self, reference_space: &Spatial, p: Vec3A, r: f32) -> Vec3A {
		let reference_to_local_space =
			Spatial::space_to_space_matrix(Some(reference_space), Some(self.spatial_ref()));
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
				let distance = field.distance(reference_space.as_ref(), point.into());
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
				let normal = field.normal(reference_space.as_ref(), point.into(), 0.001_f32);
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
				let closest_point =
					field.closest_point(reference_space.as_ref(), point.into(), 0.001_f32);
				Ok(FlexBuffable::from(mint::Vector3::from(closest_point)).build_singleton())
			});
	}

	fn spatial_ref(&self) -> &Spatial;
}

pub enum Field {
	Box(BoxField),
	Cylinder(CylinderField),
	Sphere(SphereField),
}

impl Deref for Field {
	type Target = dyn FieldTrait;
	fn deref(&self) -> &Self::Target {
		match self {
			Field::Box(field) => field,
			Field::Cylinder(field) => field,
			Field::Sphere(field) => field,
		}
	}
}

pub struct BoxField {
	space: Rc<Spatial>,
	size: Cell<Vec3>,
}

impl BoxField {
	pub fn add_to(node: &RcCell<Node>, size: Vec3) -> Result<()> {
		ensure!(
			node.borrow().spatial.is_some(),
			"Internal: Node does not have a spatial attached!"
		);
		ensure!(
			node.borrow().field.is_none(),
			"Internal: Node already has a field attached!"
		);
		let box_field = BoxField {
			space: node.borrow().spatial.as_ref().unwrap().clone(),
			size: Cell::new(size),
		};
		box_field.add_field_methods(node);
		node.borrow_mut()
			.add_local_signal("setSize", BoxField::set_size_flex);
		node.borrow_mut().field = Some(Rc::new(Field::Box(box_field)));
		Ok(())
	}

	pub fn set_size(&self, size: Vec3) {
		self.size.set(size);
	}

	pub fn set_size_flex(node: &Node, _calling_client: Rc<Client>, data: &[u8]) -> Result<()> {
		let root = flexbuffers::Reader::get_root(data)?;
		let size = flex_to_vec3!(root).ok_or_else(|| anyhow!("Size is invalid"))?;
		let field = node
			.field
			.as_ref()
			.ok_or_else(|| anyhow!("Node does not have a field"))?;
		if let Field::Box(box_field) = field.as_ref() {
			box_field.set_size(size.into());
		}
		Ok(())
	}
}

impl FieldTrait for BoxField {
	fn local_distance(&self, p: Vec3A) -> f32 {
		let size = self.size.get();
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

pub struct CylinderField {
	space: Rc<Spatial>,
	length: Cell<f32>,
	radius: Cell<f32>,
}

impl CylinderField {
	pub fn add_to(node: &RcCell<Node>, length: f32, radius: f32) -> Result<()> {
		ensure!(
			node.borrow().spatial.is_some(),
			"Internal: Node does not have a spatial attached!"
		);
		ensure!(
			node.borrow().field.is_none(),
			"Internal: Node already has a field attached!"
		);
		let cylinder_field = CylinderField {
			space: node.borrow().spatial.as_ref().unwrap().clone(),
			length: Cell::new(length),
			radius: Cell::new(radius),
		};
		cylinder_field.add_field_methods(node);
		node.borrow_mut()
			.add_local_signal("setSize", CylinderField::set_size_flex);
		node.borrow_mut().field = Some(Rc::new(Field::Cylinder(cylinder_field)));
		Ok(())
	}

	pub fn set_size(&self, length: f32, radius: f32) {
		self.length.set(length);
		self.radius.set(radius);
	}

	pub fn set_size_flex(node: &Node, _calling_client: Rc<Client>, data: &[u8]) -> Result<()> {
		let root = flexbuffers::Reader::get_root(data)?;
		let flex_vec = root.get_vector()?;
		let length = flex_vec.idx(0).as_f32();
		let radius = flex_vec.idx(1).as_f32();
		let field = node
			.field
			.as_ref()
			.ok_or_else(|| anyhow!("Node does not have a field"))?;
		if let Field::Cylinder(cylinder_field) = field.as_ref() {
			cylinder_field.set_size(length, radius);
		}
		Ok(())
	}
}

impl FieldTrait for CylinderField {
	fn local_distance(&self, p: Vec3A) -> f32 {
		let d = vec2(
			p.xy().length().abs() - self.radius.get(),
			p.z.abs() - (self.length.get() * 0.5),
		);

		d.x.max(d.y).min(0_f32)
			+ (if d.x >= 0_f32 && d.y >= 0_f32 {
				d.length()
			} else {
				0_f32
			})
	}
	fn spatial_ref(&self) -> &Spatial {
		self.space.as_ref()
	}
}

pub struct SphereField {
	space: Rc<Spatial>,
	radius: Cell<f32>,
}

impl SphereField {
	pub fn add_to(node: &RcCell<Node>, radius: f32) -> Result<()> {
		ensure!(
			node.borrow().spatial.is_some(),
			"Internal: Node does not have a spatial attached!"
		);
		ensure!(
			node.borrow().field.is_none(),
			"Internal: Node already has a field attached!"
		);
		let sphere_field = SphereField {
			space: node.borrow().spatial.as_ref().unwrap().clone(),
			radius: Cell::new(radius),
		};
		sphere_field.add_field_methods(node);
		node.borrow_mut()
			.add_local_signal("setRadius", SphereField::set_radius_flex);
		node.borrow_mut().field = Some(Rc::new(Field::Sphere(sphere_field)));
		Ok(())
	}

	pub fn set_radius(&self, radius: f32) {
		self.radius.set(radius);
	}

	pub fn set_radius_flex(node: &Node, _calling_client: Rc<Client>, data: &[u8]) -> Result<()> {
		let root = flexbuffers::Reader::get_root(data)?;
		let field = node
			.field
			.as_ref()
			.ok_or_else(|| anyhow!("Node does not have a field"))?;
		if let Field::Sphere(sphere_field) = field.as_ref() {
			sphere_field.set_radius(root.as_f32());
		}
		Ok(())
	}
}

impl FieldTrait for SphereField {
	fn local_distance(&self, p: Vec3A) -> f32 {
		p.length() - self.radius.get()
	}
	fn local_normal(&self, p: Vec3A, _r: f32) -> Vec3A {
		-p.normalize()
	}
	fn local_closest_point(&self, p: Vec3A, _r: f32) -> Vec3A {
		p.normalize() * self.radius.get()
	}
	fn spatial_ref(&self) -> &Spatial {
		self.space.as_ref()
	}
}

pub fn create_interface(client: Rc<Client>) {
	let mut node = Node::create(Rc::downgrade(&client), "", "field", false);
	node.add_local_signal("createBoxField", create_box_field_flex);
	node.add_local_signal("createCylinderField", create_cylinder_field_flex);
	node.add_local_signal("createSphereField", create_sphere_field_flex);
	client.get_scenegraph().add_node(node);
}

pub fn create_box_field_flex(_node: &Node, calling_client: Rc<Client>, data: &[u8]) -> Result<()> {
	let root = flexbuffers::Reader::get_root(data)?;
	let flex_vec = root.get_vector()?;
	let node = Node::create(
		Rc::downgrade(&calling_client),
		"/field",
		flex_vec.idx(0).get_str()?,
		true,
	);
	let parent = calling_client
		.get_scenegraph()
		.get_node(flex_vec.idx(1).as_str())
		.and_then(|node| node.borrow().spatial.clone());
	let transform = Mat4::from_rotation_translation(
		flex_to_quat!(flex_vec.idx(3))
			.ok_or_else(|| anyhow!("Rotation not found"))?
			.into(),
		flex_to_vec3!(flex_vec.idx(2))
			.ok_or_else(|| anyhow!("Position not found"))?
			.into(),
	);
	let size = flex_to_vec3!(flex_vec.idx(4)).ok_or_else(|| anyhow!("Size invalid"))?;
	let node_rc = calling_client.get_scenegraph().add_node(node);
	Spatial::add_to(&node_rc, parent, transform)?;
	BoxField::add_to(&node_rc, size.into())?;
	Ok(())
}

pub fn create_cylinder_field_flex(
	_node: &Node,
	calling_client: Rc<Client>,
	data: &[u8],
) -> Result<()> {
	let root = flexbuffers::Reader::get_root(data)?;
	let flex_vec = root.get_vector()?;
	let node = Node::create(
		Rc::downgrade(&calling_client),
		"/field",
		flex_vec.idx(0).get_str()?,
		true,
	);
	let parent = calling_client
		.get_scenegraph()
		.get_node(flex_vec.idx(1).as_str())
		.and_then(|node| node.borrow().spatial.clone());
	let transform = Mat4::from_rotation_translation(
		flex_to_quat!(flex_vec.idx(3))
			.ok_or_else(|| anyhow!("Rotation not found"))?
			.into(),
		flex_to_vec3!(flex_vec.idx(2))
			.ok_or_else(|| anyhow!("Position not found"))?
			.into(),
	);
	let length = flex_vec.idx(0).as_f32();
	let radius = flex_vec.idx(1).as_f32();
	let node_rc = calling_client.get_scenegraph().add_node(node);
	Spatial::add_to(&node_rc, parent, transform)?;
	CylinderField::add_to(&node_rc, length, radius)?;
	Ok(())
}

pub fn create_sphere_field_flex(
	_node: &Node,
	calling_client: Rc<Client>,
	data: &[u8],
) -> Result<()> {
	let root = flexbuffers::Reader::get_root(data)?;
	let flex_vec = root.get_vector()?;
	let node = Node::create(
		Rc::downgrade(&calling_client),
		"/field",
		flex_vec.idx(0).get_str()?,
		true,
	);
	let parent = calling_client
		.get_scenegraph()
		.get_node(flex_vec.idx(1).as_str())
		.and_then(|node| node.borrow().spatial.clone());
	let transform = Mat4::from_translation(
		flex_to_vec3!(flex_vec.idx(2))
			.ok_or_else(|| anyhow!("Position not found"))?
			.into(),
	);
	let node_rc = calling_client.get_scenegraph().add_node(node);
	Spatial::add_to(&node_rc, parent, transform)?;
	SphereField::add_to(&node_rc, flex_vec.idx(3).as_f32())?;
	Ok(())
}

use super::core::Node;
use super::spatial::Spatial;
use crate::core::client::Client;
use anyhow::{anyhow, ensure, Result};
use glam::{swizzles::*, vec2, vec3, vec3a, Mat4, Vec3, Vec3A};
use libstardustxr::fusion::flex::FlexBuffable;
use libstardustxr::{flex_to_quat, flex_to_vec3};
use parking_lot::Mutex;
use portable_atomic::AtomicF32;
use std::ops::Deref;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::sync::Arc;

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

	fn add_field_methods(&self, node: &Arc<Node>) {
		node.add_local_method("distance", field_distance_flex);
		node.add_local_method("normal", field_normal_flex);
		node.add_local_method("closest_point", field_closest_point_flex);
	}

	fn spatial_ref(&self) -> &Spatial;
}

fn field_distance_flex(node: &Node, calling_client: Rc<Client>, data: &[u8]) -> Result<Vec<u8>> {
	let root = flexbuffers::Reader::get_root(data)?;
	let flex_vec = root.get_vector()?;
	let reference_space_path = flex_vec.idx(0).as_str();
	let reference_space = calling_client
		.scenegraph
		.get_node(reference_space_path)
		.ok_or_else(|| anyhow!("Reference space node does not exist"))?
		.spatial
		.read()
		.as_ref()
		.ok_or_else(|| anyhow!("Reference space node does not have a spatial"))?
		.clone();
	let point = flex_to_vec3!(flex_vec.idx(1)).ok_or_else(|| anyhow!("Point is invalid"))?;

	let distance = node
		.field
		.read()
		.as_ref()
		.unwrap()
		.distance(reference_space.as_ref(), point.into());
	Ok(FlexBuffable::from(distance).build_singleton())
}
fn field_normal_flex(node: &Node, calling_client: Rc<Client>, data: &[u8]) -> Result<Vec<u8>> {
	let root = flexbuffers::Reader::get_root(data)?;
	let flex_vec = root.get_vector()?;
	let reference_space_path = flex_vec.idx(0).as_str();
	let reference_space = calling_client
		.scenegraph
		.get_node(reference_space_path)
		.ok_or_else(|| anyhow!("Reference space node does not exist"))?
		.spatial
		.read()
		.as_ref()
		.ok_or_else(|| anyhow!("Reference space node does not have a spatial"))?
		.clone();
	let point = flex_to_vec3!(flex_vec.idx(1)).ok_or_else(|| anyhow!("Point is invalid"))?;

	let normal = node.field.read().as_ref().unwrap().normal(
		reference_space.as_ref(),
		point.into(),
		0.001_f32,
	);
	Ok(FlexBuffable::from(mint::Vector3::from(normal)).build_singleton())
}
fn field_closest_point_flex(
	node: &Node,
	calling_client: Rc<Client>,
	data: &[u8],
) -> Result<Vec<u8>> {
	let root = flexbuffers::Reader::get_root(data)?;
	let flex_vec = root.get_vector()?;
	let reference_space_path = flex_vec.idx(0).as_str();
	let reference_space = calling_client
		.scenegraph
		.get_node(reference_space_path)
		.ok_or_else(|| anyhow!("Reference space node does not exist"))?
		.spatial
		.read()
		.as_ref()
		.ok_or_else(|| anyhow!("Reference space node does not have a spatial"))?
		.clone();
	let point = flex_to_vec3!(flex_vec.idx(1)).ok_or_else(|| anyhow!("Point is invalid"))?;

	let closest_point = node.field.read().as_ref().unwrap().closest_point(
		reference_space.as_ref(),
		point.into(),
		0.001_f32,
	);
	Ok(FlexBuffable::from(mint::Vector3::from(closest_point)).build_singleton())
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
	space: Arc<Spatial>,
	size: Mutex<Vec3>,
}

impl BoxField {
	pub fn add_to(node: &Arc<Node>, size: Vec3) -> Result<()> {
		ensure!(
			node.spatial.read().is_some(),
			"Internal: Node does not have a spatial attached!"
		);
		ensure!(
			node.field.read().is_none(),
			"Internal: Node already has a field attached!"
		);
		let box_field = BoxField {
			space: node.spatial.read().as_ref().unwrap().clone(),
			size: Mutex::new(size),
		};
		box_field.add_field_methods(node);
		node.add_local_signal("setSize", BoxField::set_size_flex);
		*node.field.write() = Some(Arc::new(Field::Box(box_field)));
		Ok(())
	}

	pub fn set_size(&self, size: Vec3) {
		*self.size.lock() = size;
	}

	pub fn set_size_flex(node: &Node, _calling_client: Rc<Client>, data: &[u8]) -> Result<()> {
		let root = flexbuffers::Reader::get_root(data)?;
		let size = flex_to_vec3!(root).ok_or_else(|| anyhow!("Size is invalid"))?;
		if let Field::Box(box_field) = node.field.read().as_ref().unwrap().as_ref() {
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

pub struct CylinderField {
	space: Arc<Spatial>,
	length: AtomicF32,
	radius: AtomicF32,
}

impl CylinderField {
	pub fn add_to(node: &Arc<Node>, length: f32, radius: f32) -> Result<()> {
		ensure!(
			node.spatial.read().is_some(),
			"Internal: Node does not have a spatial attached!"
		);
		ensure!(
			node.field.read().is_none(),
			"Internal: Node already has a field attached!"
		);
		let cylinder_field = CylinderField {
			space: node.spatial.read().as_ref().unwrap().clone(),
			length: AtomicF32::new(length),
			radius: AtomicF32::new(radius),
		};
		cylinder_field.add_field_methods(node);
		node.add_local_signal("setSize", CylinderField::set_size_flex);
		*node.field.write() = Some(Arc::new(Field::Cylinder(cylinder_field)));
		Ok(())
	}

	pub fn set_size(&self, length: f32, radius: f32) {
		self.length.store(length, Ordering::Relaxed);
		self.radius.store(radius, Ordering::Relaxed);
	}

	pub fn set_size_flex(node: &Node, _calling_client: Rc<Client>, data: &[u8]) -> Result<()> {
		let root = flexbuffers::Reader::get_root(data)?;
		let flex_vec = root.get_vector()?;
		let length = flex_vec.idx(0).as_f32();
		let radius = flex_vec.idx(1).as_f32();
		if let Field::Cylinder(cylinder_field) = node.field.read().as_ref().unwrap().as_ref() {
			cylinder_field.set_size(length, radius);
		}
		Ok(())
	}
}

impl FieldTrait for CylinderField {
	fn local_distance(&self, p: Vec3A) -> f32 {
		let radius = self.length.load(Ordering::Relaxed);
		let d = vec2(p.xy().length().abs() - radius, p.z.abs() - (radius * 0.5));

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
	space: Arc<Spatial>,
	radius: AtomicF32,
}

impl SphereField {
	pub fn add_to(node: &Arc<Node>, radius: f32) -> Result<()> {
		ensure!(
			node.spatial.read().is_some(),
			"Internal: Node does not have a spatial attached!"
		);
		ensure!(
			node.field.read().is_none(),
			"Internal: Node already has a field attached!"
		);
		let sphere_field = SphereField {
			space: node.spatial.read().as_ref().unwrap().clone(),
			radius: AtomicF32::new(radius),
		};
		sphere_field.add_field_methods(node);
		node.add_local_signal("setRadius", SphereField::set_radius_flex);
		*node.field.write() = Some(Arc::new(Field::Sphere(sphere_field)));
		Ok(())
	}

	pub fn set_radius(&self, radius: f32) {
		self.radius.store(radius, Ordering::Relaxed);
	}

	pub fn set_radius_flex(node: &Node, _calling_client: Rc<Client>, data: &[u8]) -> Result<()> {
		let root = flexbuffers::Reader::get_root(data)?;
		if let Field::Sphere(sphere_field) = node.field.read().as_ref().unwrap().as_ref() {
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

pub fn create_interface(client: Rc<Client>) {
	let node = Node::create("", "field", false);
	node.add_local_signal("createBoxField", create_box_field_flex);
	node.add_local_signal("createCylinderField", create_cylinder_field_flex);
	node.add_local_signal("createSphereField", create_sphere_field_flex);
	client.scenegraph.add_node(node);
}

pub fn create_box_field_flex(_node: &Node, calling_client: Rc<Client>, data: &[u8]) -> Result<()> {
	let root = flexbuffers::Reader::get_root(data)?;
	let flex_vec = root.get_vector()?;
	let node = Node::create("/field", flex_vec.idx(0).get_str()?, true);
	let parent = calling_client
		.scenegraph
		.get_node(flex_vec.idx(1).as_str())
		.and_then(|node| node.spatial.read().clone());
	let transform = Mat4::from_rotation_translation(
		flex_to_quat!(flex_vec.idx(3))
			.ok_or_else(|| anyhow!("Rotation not found"))?
			.into(),
		flex_to_vec3!(flex_vec.idx(2))
			.ok_or_else(|| anyhow!("Position not found"))?
			.into(),
	);
	let size = flex_to_vec3!(flex_vec.idx(4)).ok_or_else(|| anyhow!("Size invalid"))?;
	let node_rc = calling_client.scenegraph.add_node(node);
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
	let node = Node::create("/field", flex_vec.idx(0).get_str()?, true);
	let parent = calling_client
		.scenegraph
		.get_node(flex_vec.idx(1).as_str())
		.and_then(|node| node.spatial.read().clone());
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
	let node_rc = calling_client.scenegraph.add_node(node);
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
	let node = Node::create("/field", flex_vec.idx(0).get_str()?, true);
	let parent = calling_client
		.scenegraph
		.get_node(flex_vec.idx(1).as_str())
		.and_then(|node| node.spatial.read().clone());
	let transform = Mat4::from_translation(
		flex_to_vec3!(flex_vec.idx(2))
			.ok_or_else(|| anyhow!("Position not found"))?
			.into(),
	);
	let node_rc = calling_client.scenegraph.add_node(node);
	Spatial::add_to(&node_rc, parent, transform)?;
	SphereField::add_to(&node_rc, flex_vec.idx(3).as_f32())?;
	Ok(())
}

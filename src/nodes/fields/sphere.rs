use super::{Field, FieldTrait, Node};
use crate::core::client::Client;
use crate::nodes::spatial::{find_spatial_parent, Spatial};
use crate::nodes::Message;
use color_eyre::eyre::{ensure, Result};
use glam::{Mat4, Vec3A};
use mint::Vector3;
use portable_atomic::AtomicF32;
use serde::Deserialize;
use stardust_xr::schemas::flex::deserialize;
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
		node.add_local_signal("set_radius", SphereField::set_radius_flex);
		let _ = node.field.set(Arc::new(Field::Sphere(sphere_field)));
		Ok(())
	}

	pub fn set_radius(&self, radius: f32) {
		self.radius.store(radius, Ordering::Relaxed);
	}

	pub fn set_radius_flex(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		message: Message,
	) -> Result<()> {
		let Field::Sphere(sphere_field) = node.field.get().unwrap().as_ref() else {
			return Ok(());
		};
		sphere_field.set_radius(deserialize(message.as_ref())?);
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
	_node: Arc<Node>,
	calling_client: Arc<Client>,
	message: Message,
) -> Result<()> {
	#[derive(Deserialize)]
	struct CreateFieldInfo<'a> {
		name: &'a str,
		parent_path: &'a str,
		origin: Option<Vector3<f32>>,
		radius: f32,
	}
	let info: CreateFieldInfo = deserialize(message.as_ref())?;
	let node = Node::create_parent_name(&calling_client, "/field", info.name, true);
	let parent = find_spatial_parent(&calling_client, info.parent_path)?;
	let transform = Mat4::from_translation(
		info.origin
			.unwrap_or_else(|| Vector3::from([0.0; 3]))
			.into(),
	);
	let node = node.add_to_scenegraph()?;
	Spatial::add_to(&node, Some(parent), transform, false)?;
	SphereField::add_to(&node, info.radius)?;
	Ok(())
}

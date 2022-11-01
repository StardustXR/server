mod r#box;
mod cylinder;
mod sphere;

use self::cylinder::{create_cylinder_field_flex, CylinderField};
use self::r#box::{create_box_field_flex, BoxField};
use self::sphere::{create_sphere_field_flex, SphereField};

use super::spatial::Spatial;
use super::Node;
use crate::core::client::Client;
use crate::nodes::spatial::find_reference_space;
use anyhow::Result;
use glam::{vec2, vec3a, Vec3, Vec3A};
use mint::Vector3;
use serde::{Deserialize, Serialize};
use stardust_xr::schemas::flex::{deserialize, serialize};

use std::ops::Deref;
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
		node.add_local_method("ray_march", field_ray_march_flex);
	}

	fn spatial_ref(&self) -> &Spatial;
}

fn field_distance_flex(node: &Node, calling_client: Arc<Client>, data: &[u8]) -> Result<Vec<u8>> {
	#[derive(Deserialize)]
	struct FieldInfoArgs<'a> {
		reference_space_path: &'a str,
		point: Vector3<f32>,
	}
	let args: FieldInfoArgs = deserialize(data)?;
	let reference_space = find_reference_space(&calling_client, args.reference_space_path)?;

	let distance = node
		.field
		.get()
		.unwrap()
		.distance(reference_space.as_ref(), args.point.into());
	Ok(serialize(distance)?)
}
fn field_normal_flex(node: &Node, calling_client: Arc<Client>, data: &[u8]) -> Result<Vec<u8>> {
	#[derive(Deserialize)]
	struct FieldInfoArgs<'a> {
		reference_space_path: &'a str,
		point: Vector3<f32>,
		radius: Option<f32>,
	}
	let args: FieldInfoArgs = deserialize(data)?;
	let reference_space = find_reference_space(&calling_client, args.reference_space_path)?;

	let normal = node.field.get().as_ref().unwrap().normal(
		reference_space.as_ref(),
		args.point.into(),
		args.radius.unwrap_or(0.001),
	);
	Ok(serialize(mint::Vector3::from(normal))?)
}
fn field_closest_point_flex(
	node: &Node,
	calling_client: Arc<Client>,
	data: &[u8],
) -> Result<Vec<u8>> {
	#[derive(Deserialize)]
	struct FieldInfoArgs<'a> {
		reference_space_path: &'a str,
		point: Vector3<f32>,
		radius: Option<f32>,
	}
	let args: FieldInfoArgs = deserialize(data)?;
	let reference_space = find_reference_space(&calling_client, args.reference_space_path)?;

	let closest_point = node.field.get().as_ref().unwrap().closest_point(
		reference_space.as_ref(),
		args.point.into(),
		args.radius.unwrap_or(0.001),
	);
	Ok(serialize(mint::Vector3::from(closest_point))?)
}
fn field_ray_march_flex(node: &Node, calling_client: Arc<Client>, data: &[u8]) -> Result<Vec<u8>> {
	#[derive(Deserialize)]
	struct FieldInfoArgs<'a> {
		reference_space_path: &'a str,
		ray_origin: Vector3<f32>,
		ray_direction: Vector3<f32>,
	}
	let args: FieldInfoArgs = deserialize(data)?;
	let reference_space = find_reference_space(&calling_client, args.reference_space_path)?;

	let ray_march_result = ray_march(
		Ray {
			origin: args.ray_origin.into(),
			direction: args.ray_direction.into(),
			space: reference_space,
		},
		node.field.get().unwrap(),
	);
	Ok(serialize(ray_march_result)?)
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

pub fn create_interface(client: &Arc<Client>) {
	let node = Node::create(client, "", "field", false);
	node.add_local_signal("createBoxField", create_box_field_flex);
	node.add_local_signal("createCylinderField", create_cylinder_field_flex);
	node.add_local_signal("createSphereField", create_sphere_field_flex);
	node.add_to_scenegraph();
}

pub struct Ray {
	pub origin: Vec3,
	pub direction: Vec3,
	pub space: Arc<Spatial>,
}

#[derive(Debug, Serialize)]
pub struct RayMarchResult {
	pub min_distance: f32,
	pub deepest_point_distance: f32,
	pub ray_length: f32,
	pub ray_steps: u32,
}

// const MIN_RAY_STEPS: u32 = 0;
const MAX_RAY_STEPS: u32 = 1000;

const MIN_RAY_MARCH: f32 = 0.001_f32;
const MAX_RAY_MARCH: f32 = f32::MAX;

// const MIN_RAY_LENGTH: f32 = 0_f32;
const MAX_RAY_LENGTH: f32 = 1000_f32;

pub fn ray_march(ray: Ray, field: &Field) -> RayMarchResult {
	let mut result = RayMarchResult {
		min_distance: f32::MAX,
		deepest_point_distance: 0_f32,
		ray_length: 0_f32,
		ray_steps: 0,
	};

	let ray_to_field_matrix =
		Spatial::space_to_space_matrix(Some(&ray.space), Some(field.spatial_ref()));
	let mut ray_point = ray_to_field_matrix.transform_point3a(ray.origin.into());
	let ray_direction = ray_to_field_matrix.transform_vector3a(ray.direction.into());

	while result.ray_steps < MAX_RAY_STEPS && result.ray_length < MAX_RAY_LENGTH {
		let distance = field.local_distance(ray_point);
		let march_distance = distance.clamp(MIN_RAY_MARCH, MAX_RAY_MARCH);

		result.ray_length += march_distance;
		ray_point += ray_direction * march_distance;

		if result.min_distance > distance {
			result.deepest_point_distance = result.ray_length;
		}
		result.min_distance = distance.min(result.min_distance);

		result.ray_steps += 1;
	}

	result
}

pub fn find_field(client: &Client, path: &str) -> Result<Arc<Field>> {
	client
		.get_node("Field", path)?
		.get_aspect("Field", "info", |n| &n.field)
		.cloned()
}

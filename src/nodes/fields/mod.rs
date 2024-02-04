pub mod r#box;
mod cylinder;
mod sphere;
mod torus;

use self::cylinder::CylinderField;
use self::r#box::BoxField;
use self::sphere::SphereField;
use self::torus::TorusField;

use super::alias::AliasInfo;
use super::spatial::{get_spatial, Spatial};
use super::Node;
use crate::core::client::Client;
use crate::create_interface;
use crate::nodes::spatial::Transform;
use color_eyre::eyre::Result;
use glam::{vec2, vec3a, Mat4, Vec3, Vec3A};
use mint::Vector3;
use once_cell::sync::Lazy;
use std::ops::Deref;
use std::sync::Arc;

// TODO: get SDFs working properly with non-uniform scale and so on, output distance relative to the spatial it's compared against

pub static FIELD_ALIAS_INFO: Lazy<AliasInfo> = Lazy::new(|| AliasInfo {
	server_methods: vec!["distance", "normal", "closest_point", "ray_march"],
	..Default::default()
});

stardust_xr_server_codegen::codegen_field_protocol!();

pub trait FieldTrait {
	fn spatial_ref(&self) -> &Spatial;

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

	fn ray_march(&self, ray: Ray) -> RayMarchResult {
		let mut result = RayMarchResult {
			ray_origin: ray.origin.into(),
			ray_direction: ray.direction.into(),
			min_distance: f32::MAX,
			deepest_point_distance: 0_f32,
			ray_length: 0_f32,
			ray_steps: 0,
		};

		let ray_to_field_matrix =
			Spatial::space_to_space_matrix(Some(&ray.space), Some(self.spatial_ref()));
		let mut ray_point = ray_to_field_matrix.transform_point3a(ray.origin.into());
		let ray_direction = ray_to_field_matrix.transform_vector3a(ray.direction.into());

		while result.ray_steps < MAX_RAY_STEPS && result.ray_length < MAX_RAY_LENGTH {
			let distance = self.local_distance(ray_point);
			let march_distance = distance.clamp(MIN_RAY_MARCH, MAX_RAY_MARCH);

			result.ray_length += march_distance;
			ray_point += ray_direction * march_distance;

			if result.min_distance > distance {
				result.deepest_point_distance = result.ray_length;
				result.min_distance = distance;
			}

			result.ray_steps += 1;
		}

		result
	}
}
impl<Fi: FieldTrait + 'static> FieldAspect for Fi {
	async fn distance(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		space: Arc<Node>,
		point: mint::Vector3<f32>,
	) -> Result<f32> {
		let reference_space = get_spatial(&space, "Reference space")?;
		let this_field = node.field.get().unwrap();
		Ok(this_field.distance(reference_space.as_ref(), point.into()))
	}

	async fn normal(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		space: Arc<Node>,
		point: mint::Vector3<f32>,
	) -> Result<Vector3<f32>> {
		let reference_space = get_spatial(&space, "Reference space")?;
		let this_field = node.field.get().unwrap();
		Ok(this_field
			.normal(reference_space.as_ref(), point.into(), 0.001)
			.into())
	}

	async fn closest_point(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		space: Arc<Node>,
		point: mint::Vector3<f32>,
	) -> Result<Vector3<f32>> {
		let reference_space = get_spatial(&space, "Reference space")?;
		let this_field = node.field.get().unwrap();
		Ok(this_field
			.closest_point(reference_space.as_ref(), point.into(), 0.001)
			.into())
	}

	async fn ray_march(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		space: Arc<Node>,
		ray_origin: mint::Vector3<f32>,
		ray_direction: mint::Vector3<f32>,
	) -> Result<RayMarchResult> {
		let reference_space = get_spatial(&space, "Reference space")?;
		let this_field = node.field.get().unwrap();
		Ok(this_field.ray_march(Ray {
			origin: ray_origin.into(),
			direction: ray_direction.into(),
			space: reference_space,
		}))
	}
}

pub struct Ray {
	pub origin: Vec3,
	pub direction: Vec3,
	pub space: Arc<Spatial>,
}

// const MIN_RAY_STEPS: u32 = 0;
const MAX_RAY_STEPS: u32 = 1000;

const MIN_RAY_MARCH: f32 = 0.001_f32;
const MAX_RAY_MARCH: f32 = f32::MAX;

// const MIN_RAY_LENGTH: f32 = 0_f32;
const MAX_RAY_LENGTH: f32 = 1000_f32;

pub enum Field {
	Box(BoxField),
	Cylinder(CylinderField),
	Sphere(SphereField),
	Torus(TorusField),
}
impl Deref for Field {
	type Target = dyn FieldTrait;
	fn deref(&self) -> &Self::Target {
		match self {
			Field::Box(field) => field,
			Field::Cylinder(field) => field,
			Field::Sphere(field) => field,
			Field::Torus(field) => field,
		}
	}
}

create_interface!(FieldInterface, FieldInterfaceAspect, "/field");
pub struct FieldInterface;
impl FieldInterfaceAspect for FieldInterface {
	fn create_box_field(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		name: String,
		parent: Arc<Node>,
		transform: Transform,
		size: mint::Vector3<f32>,
	) -> Result<()> {
		let transform = transform.to_mat4(true, true, false);
		let parent = get_spatial(&parent, "Spatial parent")?;
		let node = Node::create_parent_name(
			&calling_client,
			Self::CREATE_BOX_FIELD_PARENT_PATH,
			&name,
			true,
		)
		.add_to_scenegraph()?;
		Spatial::add_to(&node, Some(parent), transform, false)?;
		BoxField::add_to(&node, size)?;
		Ok(())
	}

	fn create_cylinder_field(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		name: String,
		parent: Arc<Node>,
		transform: Transform,
		length: f32,
		radius: f32,
	) -> Result<()> {
		let transform = transform.to_mat4(true, true, false);
		let parent = get_spatial(&parent, "Spatial parent")?;
		let node = Node::create_parent_name(
			&calling_client,
			Self::CREATE_CYLINDER_FIELD_PARENT_PATH,
			&name,
			true,
		)
		.add_to_scenegraph()?;
		Spatial::add_to(&node, Some(parent), transform, false)?;
		CylinderField::add_to(&node, length, radius)?;
		Ok(())
	}

	fn create_sphere_field(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		name: String,
		parent: Arc<Node>,
		position: mint::Vector3<f32>,
		radius: f32,
	) -> Result<()> {
		let parent = get_spatial(&parent, "Spatial parent")?;
		let node = Node::create_parent_name(
			&calling_client,
			Self::CREATE_SPHERE_FIELD_PARENT_PATH,
			&name,
			true,
		)
		.add_to_scenegraph()?;
		Spatial::add_to(
			&node,
			Some(parent),
			Mat4::from_translation(position.into()),
			false,
		)?;
		SphereField::add_to(&node, radius)?;
		Ok(())
	}

	fn create_torus_field(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		name: String,
		parent: Arc<Node>,
		transform: Transform,
		radius_a: f32,
		radius_b: f32,
	) -> Result<()> {
		let transform = transform.to_mat4(true, true, false);
		let parent = get_spatial(&parent, "Spatial parent")?;
		let node = Node::create_parent_name(
			&calling_client,
			Self::CREATE_TORUS_FIELD_PARENT_PATH,
			&name,
			true,
		)
		.add_to_scenegraph()?;
		Spatial::add_to(&node, Some(parent), transform, false)?;
		TorusField::add_to(&node, radius_a, radius_b)?;
		Ok(())
	}
}

pub fn find_field(client: &Client, path: &str) -> Result<Arc<Field>> {
	client
		.get_node("Field", path)?
		.get_aspect("Field", "info", |n| &n.field)
		.cloned()
}
pub fn get_field(node: &Node) -> Result<Arc<Field>> {
	node.get_aspect("Field", "info", |n| &n.field).cloned()
}

use super::alias::{Alias, AliasInfo};
use super::spatial::{
	SPATIAL_REF_GET_LOCAL_BOUNDING_BOX_SERVER_OPCODE,
	SPATIAL_REF_GET_RELATIVE_BOUNDING_BOX_SERVER_OPCODE, SPATIAL_REF_GET_TRANSFORM_SERVER_OPCODE,
	Spatial,
};
use super::{Aspect, AspectIdentifier, Node};
use crate::DbusConnection;
use crate::core::Id;
use crate::core::client::Client;
use crate::core::error::Result;
use crate::core::registry::Registry;
use crate::nodes::spatial::SPATIAL_ASPECT_ALIAS_INFO;
use crate::nodes::spatial::SPATIAL_REF_ASPECT_ALIAS_INFO;
use crate::nodes::spatial::Transform;
use bevy::app::{Plugin, Update};
use bevy::color::Color;
use bevy::ecs::resource::Resource;
use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::ecs::system::Res;
use bevy::gizmos::gizmos::Gizmos;
use bevy::gizmos::primitives::dim3::GizmoPrimitive3d;
use bevy::math::primitives::{Cylinder, Torus};
use color_eyre::eyre::OptionExt;
use glam::{Vec3, Vec3A, Vec3Swizzles, vec2, vec3, vec3a};
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use stardust_xr_wire::values::Vector3;
use std::sync::{Arc, LazyLock, Weak};
use zbus::interface;

// TODO: get SDFs working properly with non-uniform scale and so on, output distance relative to the spatial it's compared against

pub static FIELD_ALIAS_INFO: LazyLock<AliasInfo> = LazyLock::new(|| AliasInfo {
	server_methods: vec![
		SPATIAL_REF_GET_TRANSFORM_SERVER_OPCODE,
		SPATIAL_REF_GET_LOCAL_BOUNDING_BOX_SERVER_OPCODE,
		SPATIAL_REF_GET_RELATIVE_BOUNDING_BOX_SERVER_OPCODE,
		FIELD_REF_DISTANCE_SERVER_OPCODE,
		FIELD_REF_NORMAL_SERVER_OPCODE,
		FIELD_REF_CLOSEST_POINT_SERVER_OPCODE,
		FIELD_REF_RAY_MARCH_SERVER_OPCODE,
	],
	..Default::default()
});

pub struct FieldDebugGizmoPlugin;
impl Plugin for FieldDebugGizmoPlugin {
	fn build(&self, app: &mut bevy::app::App) {
		let (tx, rx) = tokio::sync::watch::channel(false);
		let conn = app.world().resource::<DbusConnection>().0.clone();
		tokio::spawn(async move {
			_ = conn
				.object_server()
				.at("/org/stardustxr/Server", FieldDebugGizmos { state: tx })
				.await;
		});
		app.insert_resource(FieldDebugGizmosEnabled(rx));
		app.add_systems(
			Update,
			draw_field_gizmos.run_if(|res: Res<FieldDebugGizmosEnabled>| *res.0.borrow()),
		);
	}
}

#[derive(Resource)]
struct FieldDebugGizmosEnabled(tokio::sync::watch::Receiver<bool>);

fn draw_field_gizmos(mut gizmos: Gizmos) {
	FIELD_REGISTRY_DEBUG_GIZMOS
		.get_valid_contents()
		.iter()
		.for_each(|f| {
			let transform =
				bevy::transform::components::Transform::from_matrix(f.spatial.global_transform());
			let color = Color::srgb_u8(0x04, 0xFD, 0x4C);
			match f.shape.lock().clone() {
				Shape::Box(size) => gizmos.cuboid(transform.with_scale(size.into()), color),
				Shape::Cylinder(CylinderShape { length, radius }) => {
					gizmos
						.primitive_3d(
							&Cylinder {
								radius,
								half_height: length * 0.5,
							},
							transform.to_isometry(),
							color,
						)
						.resolution(32);
				}
				Shape::Sphere(radius) => {
					gizmos.sphere(transform.to_isometry(), radius, color);
				}
				Shape::Torus(TorusShape { radius_a, radius_b }) => {
					let minor_radius;
					let major_radius;
					if radius_a >= radius_b {
						major_radius = radius_a;
						minor_radius = radius_b;
					} else {
						major_radius = radius_b;
						minor_radius = radius_a;
					}
					gizmos
						.primitive_3d(
							&Torus {
								minor_radius,
								major_radius,
							},
							transform.to_isometry(),
							color,
						)
						.minor_resolution(32)
						.major_resolution(32);
				}
			}
		});
}

struct FieldDebugGizmos {
	state: tokio::sync::watch::Sender<bool>,
}

#[interface(name = "org.stardustxr.debug.FieldDebugGizmos")]
impl FieldDebugGizmos {
	fn enable(&mut self) {
		_ = self.state.send(true);
	}
	fn disable(&mut self) {
		_ = self.state.send(false);
	}
}

static FIELD_REGISTRY_DEBUG_GIZMOS: Registry<Field> = Registry::new();

stardust_xr_server_codegen::codegen_field_protocol!();

lazy_static::lazy_static! {
	pub static ref EXPORTED_FIELDS: Mutex<FxHashMap<u64, Weak<Node>>> = Mutex::new(FxHashMap::default());
}

pub trait FieldTrait: Send + Sync + 'static {
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
		let ray_direction = ray_to_field_matrix
			.transform_vector3a(ray.direction.into())
			.normalize();

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

pub struct Field {
	pub spatial: Arc<Spatial>,
	pub shape: Mutex<Shape>,
}
impl Field {
	pub fn add_to(node: &Arc<Node>, shape: Shape) -> Result<Arc<Field>> {
		let spatial = node.get_aspect::<Spatial>()?;
		let field = Field {
			spatial,
			shape: Mutex::new(shape),
		};
		let field = node.add_aspect(field);
		FIELD_REGISTRY_DEBUG_GIZMOS.add_raw(&field);
		node.add_aspect(FieldRef);
		Ok(field)
	}
}
impl Drop for Field {
	fn drop(&mut self) {
		FIELD_REGISTRY_DEBUG_GIZMOS.remove(self);
	}
}
impl AspectIdentifier for Field {
	impl_aspect_for_field_aspect_id! {}
}
impl Aspect for Field {
	impl_aspect_for_field_aspect! {}
}
impl FieldAspect for Field {
	fn set_shape(node: Arc<Node>, _calling_client: Arc<Client>, shape: Shape) -> Result<()> {
		let field = node.get_aspect::<Field>()?;
		*field.shape.lock() = shape;
		Ok(())
	}

	async fn export_field(node: Arc<Node>, _calling_client: Arc<Client>) -> Result<Id> {
		let id = rand::random();
		EXPORTED_FIELDS.lock().insert(id, Arc::downgrade(&node));
		Ok(id.into())
	}
}
impl FieldTrait for Field {
	fn spatial_ref(&self) -> &Spatial {
		&self.spatial
	}
	fn local_distance(&self, p: Vec3A) -> f32 {
		match self.shape.lock().clone() {
			Shape::Box(size) => {
				let q = vec3(
					p.x.abs() - (size.x * 0.5_f32),
					p.y.abs() - (size.y * 0.5_f32),
					p.z.abs() - (size.z * 0.5_f32),
				);
				let v = vec3a(q.x.max(0_f32), q.y.max(0_f32), q.z.max(0_f32));
				v.length() + q.x.max(q.y.max(q.z)).min(0_f32)
			}
			Shape::Cylinder(CylinderShape { length, radius }) => {
				let d = vec2(p.xz().length().abs() - radius, p.y.abs() - (length * 0.5));
				d.x.max(d.y).min(0.0) + d.max(vec2(0.0, 0.0)).length()
			}
			Shape::Sphere(radius) => p.length() - radius,
			Shape::Torus(TorusShape { radius_a, radius_b }) => {
				let q = vec2(p.xz().length() - radius_a, p.y);
				q.length() - radius_b
			}
		}
	}
}

pub struct FieldRef;
impl AspectIdentifier for FieldRef {
	impl_aspect_for_field_ref_aspect_id! {}
}
impl Aspect for FieldRef {
	impl_aspect_for_field_ref_aspect! {}
}
impl FieldRefAspect for FieldRef {
	async fn distance(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		space: Arc<Node>,
		point: Vector3<f32>,
	) -> Result<f32> {
		let reference_space = space.get_aspect::<Spatial>()?;
		let field = node.get_aspect::<Field>()?;
		Ok(field.distance(&reference_space, point.into()))
	}

	async fn normal(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		space: Arc<Node>,
		point: Vector3<f32>,
	) -> Result<Vector3<f32>> {
		let reference_space = space.get_aspect::<Spatial>()?;
		let field = node.get_aspect::<Field>()?;
		Ok(field.normal(&reference_space, point.into(), 0.0001).into())
	}

	async fn closest_point(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		space: Arc<Node>,
		point: Vector3<f32>,
	) -> Result<Vector3<f32>> {
		let reference_space = space.get_aspect::<Spatial>()?;
		let field = node.get_aspect::<Field>()?;
		Ok(field
			.closest_point(&reference_space, point.into(), 0.0001)
			.into())
	}

	async fn ray_march(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		space: Arc<Node>,
		ray_origin: Vector3<f32>,
		ray_direction: Vector3<f32>,
	) -> Result<RayMarchResult> {
		let space = space.get_aspect::<Spatial>()?;
		let field = node.get_aspect::<Field>()?;
		Ok(field.ray_march(Ray {
			origin: ray_origin.into(),
			direction: ray_direction.into(),
			space,
		}))
	}
}

impl InterfaceAspect for Interface {
	async fn import_field_ref(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		uid: Id,
	) -> Result<Id> {
		let node = EXPORTED_FIELDS
			.lock()
			.get(&uid.0)
			.and_then(|s| s.upgrade())
			.map(|s| {
				Alias::create(
					&s,
					&calling_client,
					FIELD_REF_ASPECT_ALIAS_INFO.clone(),
					None,
				)
				.unwrap()
			})
			.ok_or_eyre("Couldn't import field with that ID")?;
		Ok(node.get_id())
	}

	fn create_field(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		id: Id,
		parent: Arc<Node>,
		transform: Transform,
		shape: Shape,
	) -> Result<()> {
		let transform = transform.to_mat4(true, true, false);
		let parent = parent.get_aspect::<Spatial>()?;
		let node = Node::from_id(&calling_client, id, true).add_to_scenegraph()?;
		Spatial::add_to(&node, Some(parent.clone()), transform);
		Field::add_to(&node, shape)?;
		Ok(())
	}
}

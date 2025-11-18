pub mod zone;

use self::zone::Zone;
use super::alias::Alias;
use super::fields::{Field, FieldTrait};
use super::{Aspect, AspectIdentifier};
use crate::bail;
use crate::core::client::Client;
use crate::core::entity_handle::EntityHandle;
use crate::core::error::Result;
use crate::core::registry::Registry;
use crate::nodes::{Node, OWNED_ASPECT_ALIAS_INFO};
use bevy::ecs::entity::EntityHashMap;
use bevy::prelude::Transform as BevyTransform;
use bevy::prelude::*;
use bevy::render::primitives::Aabb;
use color_eyre::eyre::OptionExt;
use glam::{Mat4, Quat, Vec3, vec3a};
use mint::Vector3;
use parking_lot::{Mutex, RwLock};
use rustc_hash::FxHashMap;
use std::fmt::Debug;
use std::pin::Pin;
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock, Weak};
use std::{f32, ptr};

pub struct SpatialNodePlugin;
impl Plugin for SpatialNodePlugin {
	fn build(&self, app: &mut App) {
		app.add_systems(
			PostUpdate,
			(spawn_spatial_nodes, update_spatial_nodes)
				.chain()
				.before(TransformSystem::TransformPropagate),
		);
	}
}

fn spawn_spatial_nodes(mut cmds: Commands) {
	for spatial in SPATIAL_REGISTRY
		.get_valid_contents()
		.into_iter()
		.filter(|v| v.entity.read().is_none())
	{
		let entity = cmds
			.spawn((SpatialNode(Arc::downgrade(&spatial)), Name::new("Spatial")))
			.id();
		spatial.set_entity(EntityHandle::new(entity));
	}
}

fn update_spatial_nodes(
	mut query: Query<(&mut BevyTransform, &mut Visibility, Option<&ChildOf>)>,
	mut cmds: Commands,
) {
	for (entity, (transform, parent_entity)) in UPDATED_SPATIALS_NODES.lock().drain() {
		let _span = debug_span!("updating spatial node").entered();
		let Ok((mut bevy_transform, mut vis, parent)) = query.get_mut(entity) else {
			continue;
		};
		// Set visibility based on node enabled state
		if let Some(transform) = transform {
			*vis = Visibility::Inherited;
			*bevy_transform = transform;
		} else {
			*vis = Visibility::Hidden;
		}

		if parent.map(|v| v.0) != parent_entity {
			match parent_entity {
				Some(e) => cmds.entity(entity).insert(ChildOf(e)),
				None => cmds.entity(entity).remove::<ChildOf>(),
			};
		}
	}
}

static SPATIAL_REGISTRY: Registry<Spatial> = Registry::new();

#[derive(Clone, Component, Debug)]
#[require(BevyTransform, Visibility)]
pub struct SpatialNode(pub Weak<Spatial>);

const EPSILON: f32 = 0.00001;

stardust_xr_server_codegen::codegen_spatial_protocol!();
impl Transform {
	pub fn to_mat4(&self, position: bool, rotation: bool, scale: bool) -> Mat4 {
		let position = position
			.then_some(self.translation)
			.flatten()
			.unwrap_or_else(|| Vector3::from([0.0; 3]));
		let rotation = rotation
			.then_some(self.rotation)
			.flatten()
			.unwrap_or_else(|| Quat::IDENTITY.into());

		// Zero scale values break everything
		let scale = scale
			.then_some(self.scale)
			.flatten()
			.map(|s| Vector3 {
				x: if s.x == 0.0 { EPSILON } else { s.x },
				y: if s.y == 0.0 { EPSILON } else { s.y },
				z: if s.z == 0.0 { EPSILON } else { s.z },
			})
			.unwrap_or_else(|| Vector3::from([1.0; 3]));

		Mat4::from_scale_rotation_translation(scale.into(), rotation.into(), position.into())
	}
}
impl AspectIdentifier for Zone {
	impl_aspect_for_zone_aspect_id! {}
}
impl Aspect for Zone {
	impl_aspect_for_zone_aspect! {}
}

lazy_static::lazy_static! {
	pub static ref EXPORTED_SPATIALS: Mutex<FxHashMap<u64, Weak<Node>>> = Mutex::new(FxHashMap::default());
}

static ZONEABLE_REGISTRY: Registry<Spatial> = Registry::new();

pub struct Spatial {
	pub node: Weak<Node>,
	entity: RwLock<Option<EntityHandle>>,
	parent: RwLock<Option<Arc<Spatial>>>,
	old_parent: RwLock<Option<Arc<Spatial>>>,
	transform: RwLock<Mat4>,
	zone: RwLock<Weak<Zone>>,
	children: Registry<Spatial>,
	pub bounding_box_calc:
		OnceLock<for<'a> fn(&'a Node) -> Pin<Box<dyn Future<Output = Aabb> + 'a + Send + Sync>>>,
}

impl Spatial {
	pub fn new(node: Weak<Node>, parent: Option<Arc<Spatial>>, transform: Mat4) -> Arc<Self> {
		let spatial = SPATIAL_REGISTRY.add(Spatial {
			node,
			entity: RwLock::new(None),
			parent: RwLock::new(parent),
			old_parent: RwLock::new(None),
			transform: RwLock::new(transform),
			zone: RwLock::new(Weak::new()),
			children: Registry::new(),
			bounding_box_calc: OnceLock::default(),
		});
		spatial.mark_dirty();
		spatial
	}
	pub fn set_entity(&self, entity: EntityHandle) {
		self.entity.write().replace(entity);
		self.mark_dirty();
		for child in self.children.get_valid_contents() {
			child.mark_dirty();
		}
	}
	pub fn get_entity(&self) -> Option<Entity> {
		self.entity.read().as_ref().map(|v| v.get())
	}
	pub fn add_to(
		node: &Arc<Node>,
		parent: Option<Arc<Spatial>>,
		transform: Mat4,
		zoneable: bool,
	) -> Arc<Spatial> {
		let spatial = Spatial::new(Arc::downgrade(node), parent.clone(), transform);
		if zoneable {
			ZONEABLE_REGISTRY.add_raw(&spatial);
		}
		if let Some(parent) = parent {
			parent.children.add_raw(&spatial);
		}
		node.add_aspect_raw(spatial.clone());
		node.add_aspect(SpatialRef);
		spatial
	}

	pub fn node(&self) -> Option<Arc<Node>> {
		self.node.upgrade()
	}

	pub fn space_to_space_matrix(from: Option<&Spatial>, to: Option<&Spatial>) -> Mat4 {
		let space_to_world_matrix = from.map_or(Mat4::IDENTITY, |from| from.global_transform());
		let world_to_space_matrix = to.map_or(Mat4::IDENTITY, |to| to.global_transform().inverse());
		world_to_space_matrix * space_to_world_matrix
	}

	// the output bounds are probably way bigger than they need to be
	pub async fn get_bounding_box(&self) -> Aabb {
		let Some(node) = self.node() else {
			return Aabb::default();
		};
		let mut bounds = match self.bounding_box_calc.get() {
			Some(f) => f(&node).await,
			None => Aabb::default(),
		};
		for child in self.children.get_valid_contents() {
			let mat = child.local_transform();
			let child_aabb = Box::pin(child.get_bounding_box()).await;
			bounds = Aabb::enclosing([
				bounds.min().into(),
				bounds.max().into(),
				mat.transform_point3(child_aabb.min().into()),
				mat.transform_point3(child_aabb.max().into()),
			])
			.unwrap();
		}
		bounds
	}
	pub(super) fn mark_dirty(&self) {
		let Some(entity) = self.entity.read().as_ref().map(|v| v.get()) else {
			return;
		};
		let enabled = self
			.node()
			.is_none_or(|n| n.enabled.load(Ordering::Relaxed))
			&& self.local_visible();
		let transform = enabled.then(|| BevyTransform::from_matrix(self.local_transform()));
		let parent = self
			.get_parent()
			.and_then(|v| v.entity.read().as_ref().map(|v| v.get()));
		UPDATED_SPATIALS_NODES
			.lock()
			.insert(entity, (transform, parent));
	}

	pub fn local_transform(&self) -> Mat4 {
		*self.transform.read()
	}

	fn local_visible(&self) -> bool {
		// Check our own scale by looking at matrix column lengths
		let mat = self.local_transform();
		let x_scale = mat.x_axis.length_squared();
		let y_scale = mat.y_axis.length_squared();
		let z_scale = mat.z_axis.length_squared();

		x_scale >= EPSILON * 20.0 || y_scale >= EPSILON * 20.0 || z_scale >= EPSILON * 20.0
	}
	/// Check if this node or any ancestor has zero scale (for visibility culling)
	pub fn visible(&self) -> bool {
		// Check parent chain
		if let Some(parent) = self.get_parent()
			&& !parent.visible()
		{
			return false;
		}

		// Check our own scale by looking at matrix column lengths
		self.local_visible()
	}
	pub fn global_transform(&self) -> Mat4 {
		let parent_transform = self
			.get_parent()
			.as_deref()
			.map(Self::global_transform)
			.unwrap_or_default();
		parent_transform * self.local_transform()
	}
	pub fn set_local_transform(&self, transform: Mat4) {
		*self.transform.write() = transform;
		self.mark_dirty();
	}
	pub fn set_local_transform_components(
		&self,
		reference_space: Option<&Spatial>,
		transform: Transform,
	) {
		if reference_space == Some(self) {
			self.set_local_transform(transform.to_mat4(true, true, true) * self.local_transform());
			return;
		}
		let reference_to_parent_transform = reference_space
			.map(|reference_space| {
				Spatial::space_to_space_matrix(Some(reference_space), self.get_parent().as_deref())
			})
			.unwrap_or(Mat4::IDENTITY);
		let mut local_transform_in_reference_space =
			reference_to_parent_transform.inverse() * self.local_transform();
		let (mut reference_space_scl, mut reference_space_rot, mut reference_space_pos) =
			local_transform_in_reference_space.to_scale_rotation_translation();

		if let Some(pos) = transform.translation {
			reference_space_pos = pos.into()
		}
		if let Some(rot) = transform.rotation {
			reference_space_rot = rot.into()
		} else if reference_space_rot.is_nan() {
			reference_space_rot = Quat::IDENTITY;
		}
		if let Some(scl) = transform.scale {
			reference_space_scl = scl.into()
		}

		local_transform_in_reference_space = Mat4::from_scale_rotation_translation(
			reference_space_scl,
			reference_space_rot,
			reference_space_pos,
		);
		self.set_local_transform(
			reference_to_parent_transform * local_transform_in_reference_space,
		);
	}

	pub fn is_ancestor_of(&self, spatial: Arc<Spatial>) -> bool {
		let mut current_ancestor = spatial;
		loop {
			if Arc::as_ptr(&current_ancestor) == ptr::addr_of!(*self) {
				return true;
			}

			if let Some(parent) = current_ancestor.get_parent() {
				current_ancestor = parent;
			} else {
				return false;
			}
		}
	}

	fn get_parent(&self) -> Option<Arc<Spatial>> {
		self.parent.read().clone()
	}
	fn set_parent(self: &Arc<Self>, new_parent: &Arc<Spatial>) {
		if let Some(parent) = self.get_parent() {
			parent.children.remove(self);
		}
		new_parent.children.add_raw(self);

		*self.parent.write() = Some(new_parent.clone());
		self.mark_dirty();
	}

	pub fn set_spatial_parent(self: &Arc<Self>, parent: &Arc<Spatial>) -> Result<()> {
		if self.is_ancestor_of(parent.clone()) {
			bail!("Setting spatial parent would cause a loop");
		}
		self.set_parent(parent);

		Ok(())
	}
	pub fn set_spatial_parent_in_place(self: &Arc<Self>, parent: &Arc<Spatial>) -> Result<()> {
		if self.is_ancestor_of(parent.clone()) {
			bail!("Setting spatial parent would cause a loop");
		}

		self.set_local_transform(Spatial::space_to_space_matrix(Some(self), Some(parent)));
		self.set_parent(parent);

		Ok(())
	}

	pub(self) fn zone_distance(&self) -> f32 {
		self.zone
			.read()
			.upgrade()
			.map(|zone| zone.field.clone())
			.map(|field| field.distance(self, vec3a(0.0, 0.0, 0.0)))
			.unwrap_or(f32::NEG_INFINITY)
	}
}
static UPDATED_SPATIALS_NODES: Mutex<EntityHashMap<(Option<BevyTransform>, Option<Entity>)>> =
	Mutex::new(EntityHashMap::new());
impl AspectIdentifier for Spatial {
	impl_aspect_for_spatial_aspect_id! {}
}
impl Aspect for Spatial {
	impl_aspect_for_spatial_aspect! {}
}
impl SpatialAspect for Spatial {
	fn set_local_transform(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		transform: Transform,
	) -> Result<()> {
		let this_spatial = node.get_aspect::<Spatial>()?;
		this_spatial.set_local_transform_components(None, transform);
		Ok(())
	}
	fn set_relative_transform(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		relative_to: Arc<Node>,
		transform: Transform,
	) -> Result<()> {
		let this_spatial = node.get_aspect::<Spatial>()?;
		let relative_spatial = relative_to.get_aspect::<Spatial>()?;

		this_spatial.set_local_transform_components(Some(&relative_spatial), transform);
		Ok(())
	}

	fn set_spatial_parent(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		parent: Arc<Node>,
	) -> Result<()> {
		let this_spatial = node.get_aspect::<Spatial>()?;
		let parent = parent.get_aspect::<Spatial>()?;

		this_spatial.set_spatial_parent(&parent)?;
		Ok(())
	}

	fn set_spatial_parent_in_place(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		parent: Arc<Node>,
	) -> Result<()> {
		let this_spatial = node.get_aspect::<Spatial>()?;
		let parent = parent.get_aspect::<Spatial>()?;

		this_spatial.set_spatial_parent_in_place(&parent)?;
		Ok(())
	}

	fn set_zoneable(node: Arc<Node>, _calling_client: Arc<Client>, zoneable: bool) -> Result<()> {
		let spatial = node.get_aspect::<Spatial>()?;
		if zoneable {
			ZONEABLE_REGISTRY.add_raw(&spatial);
		} else {
			ZONEABLE_REGISTRY.remove(&spatial);
			zone::release(&spatial);
		}
		Ok(())
	}

	// legit gotta find a way to remove old ones, this just keeps the node alive
	async fn export_spatial(node: Arc<Node>, _calling_client: Arc<Client>) -> Result<u64> {
		let id = rand::random();
		EXPORTED_SPATIALS.lock().insert(id, Arc::downgrade(&node));
		Ok(id)
	}
}
impl PartialEq for Spatial {
	fn eq(&self, other: &Self) -> bool {
		self.node.as_ptr() == other.node.as_ptr()
	}
}
impl Debug for Spatial {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Spatial")
			.field("parent", &self.parent)
			.field("old_parent", &self.old_parent)
			.field("transform", &self.transform)
			.finish()
	}
}
impl Drop for Spatial {
	fn drop(&mut self) {
		zone::release(self);
		ZONEABLE_REGISTRY.remove(self);
		SPATIAL_REGISTRY.remove(self);
	}
}

pub struct SpatialRef;
impl AspectIdentifier for SpatialRef {
	impl_aspect_for_spatial_ref_aspect_id! {}
}
impl Aspect for SpatialRef {
	impl_aspect_for_spatial_ref_aspect! {}
}
impl SpatialRefAspect for SpatialRef {
	async fn get_local_bounding_box(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
	) -> Result<BoundingBox> {
		let this_spatial = node.get_aspect::<Spatial>()?;
		let bounds = this_spatial.get_bounding_box().await;

		Ok(BoundingBox {
			center: Vec3::from(bounds.center).into(),
			size: Vec3::from(bounds.half_extents * 2.0).into(),
		})
	}

	async fn get_relative_bounding_box(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		relative_to: Arc<Node>,
	) -> Result<BoundingBox> {
		let this_spatial = node.get_aspect::<Spatial>()?;
		let relative_spatial = relative_to.get_aspect::<Spatial>()?;
		let mat = Spatial::space_to_space_matrix(Some(&this_spatial), Some(&relative_spatial));
		let bb = this_spatial.get_bounding_box().await;
		let bounds = Aabb::enclosing([
			mat.transform_point3(bb.min().into()),
			mat.transform_point3(bb.max().into()),
		])
		.unwrap();

		Ok(BoundingBox {
			center: Vec3::from(bounds.center).into(),
			size: Vec3::from(bounds.half_extents * 2.0).into(),
		})
	}

	async fn get_transform(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		relative_to: Arc<Node>,
	) -> Result<Transform> {
		let this_spatial = node.get_aspect::<Spatial>()?;
		let relative_spatial = relative_to.get_aspect::<Spatial>()?;

		let (scale, rotation, position) = Spatial::space_to_space_matrix(
			Some(this_spatial.as_ref()),
			Some(relative_spatial.as_ref()),
		)
		.to_scale_rotation_translation();

		Ok(Transform {
			translation: Some(position.into()),
			rotation: Some(rotation.into()),
			scale: Some(scale.into()),
		})
	}
}

impl InterfaceAspect for Interface {
	fn create_spatial(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		id: u64,
		parent: Arc<Node>,
		transform: Transform,
		zoneable: bool,
	) -> Result<()> {
		let parent = parent.get_aspect::<Spatial>()?;
		let transform = transform.to_mat4(true, true, true);
		let node = Node::from_id(&calling_client, id, true).add_to_scenegraph()?;
		Spatial::add_to(&node, Some(parent.clone()), transform, zoneable);
		Ok(())
	}
	fn create_zone(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		id: u64,
		parent: Arc<Node>,
		transform: Transform,
		field: Arc<Node>,
	) -> Result<()> {
		let parent = parent.get_aspect::<Spatial>()?;
		let transform = transform.to_mat4(true, true, false);
		let field = field.get_aspect::<Field>()?;

		let node = Node::from_id(&calling_client, id, true).add_to_scenegraph()?;
		let space = Spatial::add_to(&node, Some(parent.clone()), transform, false);
		Zone::add_to(&node, space, field);
		Ok(())
	}

	async fn import_spatial_ref(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		uid: u64,
	) -> Result<Arc<Node>> {
		Ok(EXPORTED_SPATIALS
			.lock()
			.get(&uid)
			.and_then(|s| s.upgrade())
			.map(|s| {
				Alias::create(
					&s,
					&calling_client,
					SPATIAL_REF_ASPECT_ALIAS_INFO.clone(),
					None,
				)
				.unwrap()
			})
			.ok_or_eyre("Couldn't find spatial with that ID")?)
	}
}

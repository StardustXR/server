use crate::bevy_int::entity_handle::EntityHandle;
use crate::core::error::Result;
use crate::core::registry::Registry;
use crate::nodes::ProxyExt;
use crate::{impl_proxy, interface};
use bevy::ecs::entity::EntityHashMap;
use bevy::prelude::Transform as BevyTransform;
use bevy::prelude::*;
use bevy::render::primitives::Aabb;
use glam::{Mat4, Quat};
use gluon::{Handler, RefExt};
use parking_lot::Mutex;
use stardust_xr_protocol::spatial::{
	BoundingBox, CreatedSpatial, PartialTransform, Spatial as SpatialProxy, SpatialHandler,
	SpatialInterfaceHandler, SpatialRef as SpatialRefProxy, SpatialRefHandler, SpatialRefOpError,
	Transform,
};
use stardust_xr_protocol::types::CreateError;
use stardust_xr_server_foundation::bail;
use std::fmt::Debug;
use std::sync::{Arc, Weak};
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
		.filter(|v| v.entity.lock().is_none())
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

#[expect(dead_code)]
#[derive(Clone, Component, Debug)]
#[require(BevyTransform, Visibility)]
pub struct SpatialNode(pub Weak<Spatial>);

const EPSILON: f32 = 0.00001;

pub trait TransformExt {
	fn to_mat4(&self) -> Mat4;
}
pub fn aabb_corners(aabb: &Aabb) -> [Vec3; 8] {
	let min: Vec3 = aabb.min().into();
	let max: Vec3 = aabb.max().into();
	[
		Vec3::new(min.x, min.y, min.z),
		Vec3::new(min.x, min.y, max.z),
		Vec3::new(min.x, max.y, min.z),
		Vec3::new(min.x, max.y, max.z),
		Vec3::new(max.x, min.y, min.z),
		Vec3::new(max.x, min.y, max.z),
		Vec3::new(max.x, max.y, min.z),
		Vec3::new(max.x, max.y, max.z),
	]
}

fn merge_aabb(base: Aabb, other: &Aabb, transform: Option<&Mat4>) -> Aabb {
	let corners = aabb_corners(other).map(|c| transform.map_or(c, |m| m.transform_point3(c)));
	Aabb::enclosing(
		corners
			.into_iter()
			.chain([base.min().into(), base.max().into()]),
	)
	.unwrap_or(base)
}

fn clamp_scale(scale: f32) -> f32 {
	if scale.abs() <= EPSILON {
		EPSILON * scale.signum()
	} else {
		scale
	}
}
impl TransformExt for Transform {
	fn to_mat4(&self) -> Mat4 {
		// Zero scale values break everything
		Mat4::from_scale_rotation_translation(
			Vec3::from(self.scale).map(clamp_scale),
			self.rotation.into(),
			self.translation.into(),
		)
	}
}
impl TransformExt for PartialTransform {
	fn to_mat4(&self) -> Mat4 {
		Mat4::from_scale_rotation_translation(
			self.scale
				.map(|v| Vec3::from(v).map(clamp_scale))
				.unwrap_or(Vec3::ONE),
			self.rotation.map(|v| v.into()).unwrap_or(Quat::IDENTITY),
			self.translation.map(|v| v.into()).unwrap_or(Vec3::ZERO),
		)
	}
}
// impl Transform {
// 	pub fn to_mat4(&self, position: bool, rotation: bool, scale: bool) -> Mat4 {
// 		let position = position
// 			.then_some(self.translation)
// 			.flatten()
// 			.unwrap_or_else(|| Vector3::from([0.0; 3]));
// 		let rotation = rotation
// 			.then_some(self.rotation)
// 			.flatten()
// 			.unwrap_or_else(|| Quat::IDENTITY.into());
//
// 		// Zero scale values break everything
// 		let scale = scale
// 			.then_some(self.scale)
// 			.flatten()
// 			.map(|s| Vector3 {
// 				x: if s.x == 0.0 { EPSILON } else { s.x },
// 				y: if s.y == 0.0 { EPSILON } else { s.y },
// 				z: if s.z == 0.0 { EPSILON } else { s.z },
// 			})
// 			.unwrap_or_else(|| Vector3::from([1.0; 3]));
//
// 		Mat4::from_scale_rotation_translation(scale.into(), rotation.into(), position.into())
// 	}
// }

pub struct BoundingBoxCalc(Arc<dyn Fn() -> Aabb + Send + Sync + 'static>);
impl Debug for BoundingBoxCalc {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_tuple("BoundingBoxCalc").finish()
	}
}
pub struct MovedCallback(Arc<dyn Fn() + Send + Sync + 'static>);
impl Debug for MovedCallback {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_tuple("MovedCallback").finish()
	}
}

pub struct Spatial {
	entity: Mutex<Option<EntityHandle>>,
	parent: Mutex<Option<Arc<Spatial>>>,
	transform: Mutex<Mat4>,
	children: Registry<Spatial>,
	bounding_box_calc: Registry<dyn Fn() -> Aabb + Send + Sync + 'static>,
	moved_callback: Registry<dyn Fn() + Send + Sync + 'static>,
}
impl Debug for Spatial {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Spatial")
			.field("entity", &self.entity)
			.field("parent", &self.parent)
			.field("transform", &self.transform)
			.field("children", &self.children)
			.finish()
	}
}

#[derive(Deref, Handler)]
pub struct SpatialObject {
	#[deref]
	handler: Arc<Spatial>,
	spatial_ref: SpatialRefProxy,
}
impl SpatialObject {
	pub fn new(parent: Option<&Arc<Spatial>>, transform: Mat4) -> SpatialProxy {
		let handler = Arc::new(Spatial {
			entity: Mutex::new(None),
			parent: Mutex::new(parent.cloned()),
			transform: Mutex::new(transform),
			children: Registry::new(),
			bounding_box_calc: Registry::new(),
			moved_callback: Registry::new(),
		});
		if let Some(parent) = parent {
			parent.children.add_raw(&handler);
		}
		SPATIAL_REGISTRY.add_raw(&handler);
		let spatial_ref = SpatialRefProxy::new_service(SpatialRef {
			data: handler.clone(),
		})
		.unwrap();
		let (spatial_object, spatial_object_ref) = SpatialProxy::new_node(SpatialObject {
			handler,
			spatial_ref,
		})
		.unwrap();
		spatial_object.mark_dirty();
		spatial_object.to_service();
		spatial_object_ref
	}
	pub fn get_ref(&self) -> &SpatialRefProxy {
		&self.spatial_ref
	}
	pub fn spatial_arc(&self) -> &Arc<Spatial> {
		&self.handler
	}
}

impl Spatial {
	#[cfg(test)]
	pub(crate) fn test_new(parent: Option<Arc<Spatial>>, transform: Mat4) -> Arc<Spatial> {
		Arc::new(Spatial {
			entity: Mutex::new(None),
			parent: Mutex::new(parent),
			transform: Mutex::new(transform),
			children: Registry::new(),
			bounding_box_calc: Registry::new(),
			moved_callback: Registry::new(),
		})
	}
	pub fn custom_bounding_box(
		&self,
		calc: impl Fn() -> Aabb + Send + Sync + 'static,
	) -> BoundingBoxCalc {
		let arc = BoundingBoxCalc(Arc::new(calc));
		self.bounding_box_calc.add_raw(&arc.0);
		arc
	}
	pub fn moved_callback(&self, f: impl Fn() + Send + Sync + 'static) -> MovedCallback {
		let arc = MovedCallback(Arc::new(f));
		self.moved_callback.add_raw(&arc.0);
		// Propagate up the ancestor chain so moving *any* ancestor fires this callback,
		// not just this node directly. The registry keys on the callback pointer, so
		// re-adding the same Arc is idempotent, and the entries are weak — when `arc` is
		// dropped, every ancestor's entry dies on its own without explicit cleanup.
		let mut ancestor = self.get_parent();
		while let Some(node) = ancestor {
			node.moved_callback.add_raw(&arc.0);
			ancestor = node.get_parent();
		}
		arc
	}
	pub fn set_entity(&self, entity: EntityHandle) {
		self.entity.lock().replace(entity);
		self.mark_dirty();
		for child in self.children.get_valid_contents() {
			child.mark_dirty();
		}
	}
	pub fn get_entity(&self) -> Option<Entity> {
		self.entity.lock().as_ref().map(|v| v.get())
	}

	pub fn space_to_space_matrix(from: Option<&Spatial>, to: Option<&Spatial>) -> Mat4 {
		let space_to_world_matrix = from.map_or(Mat4::IDENTITY, |from| from.global_transform());
		let world_to_space_matrix = to.map_or(Mat4::IDENTITY, |to| to.global_transform().inverse());
		world_to_space_matrix * space_to_world_matrix
	}

	// the output bounds are probably way bigger than they need to be
	pub fn get_bounding_box(&self) -> Aabb {
		let mut bounds = Aabb::default();
		for f in self.bounding_box_calc.get_valid_contents() {
			bounds = merge_aabb(bounds, &f(), None);
		}
		for child in self.children.get_valid_contents() {
			let mat = child.local_transform();
			bounds = merge_aabb(bounds, &child.get_bounding_box(), Some(&mat));
		}
		bounds
	}
	pub(super) fn mark_dirty(&self) {
		let Some(entity) = self.entity.lock().as_ref().map(|v| v.get()) else {
			return;
		};
		let mat = self.local_transform();
		let transform = if Self::mat_visible(&mat) {
			Some(BevyTransform::from_matrix(mat))
		} else {
			None
		};
		let parent = if let Some(v) = self.get_parent() {
			v.entity.lock().as_ref().map(|v| v.get())
		} else {
			None
		};
		UPDATED_SPATIALS_NODES
			.lock()
			.insert(entity, (transform, parent));
	}

	pub fn local_transform(&self) -> Mat4 {
		*self.transform.lock()
	}

	fn mat_visible(mat: &Mat4) -> bool {
		mat.x_axis.length_squared() > EPSILON
			|| mat.y_axis.length_squared() > EPSILON
			|| mat.z_axis.length_squared() > EPSILON
	}

	fn local_visible(&self) -> bool {
		Self::mat_visible(&self.local_transform())
	}
	/// Check if this node or any ancestor has zero scale (for visibility culling)
	pub fn visible(&self) -> bool {
		if !self.local_visible() {
			return false;
		}
		let mut ancestor = self.get_parent();
		while let Some(node) = ancestor {
			if !node.local_visible() {
				return false;
			}
			ancestor = node.get_parent();
		}
		true
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
		*self.transform.lock() = transform;
		for f in self.moved_callback.get_valid_contents() {
			f();
		}
		self.mark_dirty();
	}
	pub fn set_local_transform_components(
		&self,
		reference_space: Option<&Spatial>,
		transform: PartialTransform,
	) {
		if reference_space.is_some_and(|reference| std::ptr::eq(reference, self)) {
			self.set_local_transform(transform.to_mat4() * self.local_transform());
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
			reference_space_scl = Vec3::from(scl).map(clamp_scale);
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
		self.parent.lock().clone()
	}
	fn set_parent(self: &Arc<Self>, new_parent: &Arc<Spatial>) {
		// This node's registry holds its own moved callbacks plus those aggregated up from
		// its entire subtree. Reparenting moves that whole set off the old ancestor chain
		// and onto the new one, so ancestor-movement notifications keep working regardless
		// of when callbacks were registered relative to parenting.
		let subtree_callbacks = self.moved_callback.get_valid_contents();
		if let Some(old_parent) = self.get_parent() {
			old_parent.children.remove(self);
			let mut ancestor = Some(old_parent);
			while let Some(node) = ancestor {
				for f in &subtree_callbacks {
					node.moved_callback.remove(&**f);
				}
				ancestor = node.get_parent();
			}
		}
		new_parent.children.add_raw(self);
		let mut ancestor = Some(new_parent.clone());
		while let Some(node) = ancestor {
			for f in &subtree_callbacks {
				node.moved_callback.add_raw(f);
			}
			ancestor = node.get_parent();
		}

		*self.parent.lock() = Some(new_parent.clone());
		// Reparenting changes this subtree's global pose even though no local transform
		// was touched, so anything watching for movement must be told.
		for f in &subtree_callbacks {
			f();
		}
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
}
static UPDATED_SPATIALS_NODES: Mutex<EntityHashMap<(Option<BevyTransform>, Option<Entity>)>> =
	Mutex::new(EntityHashMap::new());
impl SpatialHandler for SpatialObject {
	async fn spatial_ref(&self, _ctx: gluon::Context) -> SpatialRefProxy {
		self.spatial_ref.clone()
	}

	async fn get_local_bounding_box(&self, _ctx: gluon::Context) -> BoundingBox {
		let bounds = self.get_bounding_box();
		BoundingBox {
			center: bounds.center.into(),
			extents: (bounds.half_extents * 2.0).into(),
		}
	}

	async fn get_relative_bounding_box(
		&self,
		_ctx: gluon::Context,
		relative_to: SpatialRefProxy,
	) -> Result<BoundingBox, CreateError> {
		let Some(relative_to) = relative_to.owned() else {
			return Err(CreateError::InvalidRef);
		};
		let mat = Spatial::space_to_space_matrix(Some(self), Some(&relative_to));
		let bb = self.get_bounding_box();
		let bounds = Aabb::enclosing([
			mat.transform_point3(bb.min().into()),
			mat.transform_point3(bb.max().into()),
		])
		.unwrap();

		Ok(BoundingBox {
			center: Vec3::from(bounds.center).into(),
			extents: Vec3::from(bounds.half_extents * 2.0).into(),
		})
	}

	async fn get_relative_transform(
		&self,
		_ctx: gluon::Context,
		relative_to: SpatialRefProxy,
	) -> Result<Transform, CreateError> {
		let Some(relative_to) = relative_to.owned() else {
			return Err(CreateError::InvalidRef);
		};
		let (scale, rotation, position) =
			Spatial::space_to_space_matrix(Some(self), Some(&relative_to))
				.to_scale_rotation_translation();

		Ok(Transform {
			translation: position.into(),
			rotation: rotation.into(),
			scale: scale.into(),
		})
	}

	async fn set_parent(&self, _ctx: gluon::Context, parent: SpatialRefProxy) {
		let Some(parent) = parent.owned() else {
			error!("Invalid SpatialRef used as parent");
			return;
		};
		_ = self
			.set_spatial_parent(&parent)
			.inspect_err(|err| error!("error while setting spatial parent: {err}"));
	}

	async fn set_parent_in_place(&self, _ctx: gluon::Context, parent: SpatialRefProxy) {
		let Some(parent) = parent.owned() else {
			error!("Invalid SpatialRef used as parent");
			return;
		};
		_ = self
			.set_spatial_parent_in_place(&parent.data)
			.inspect_err(|err| error!("error while setting spatial parent in place: {err}"));
	}

	async fn set_local_transform(&self, _ctx: gluon::Context, transform: PartialTransform) {
		self.set_local_transform_components(None, transform);
	}

	async fn set_relative_transform(
		&self,
		_ctx: gluon::Context,
		relative_to: SpatialRefProxy,
		transform: PartialTransform,
	) {
		let Some(relative_to) = relative_to.owned() else {
			error!("Invalid SpatialRef used");
			return;
		};
		self.set_local_transform_components(Some(&relative_to), transform);
	}
}
impl Debug for SpatialObject {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Spatial")
			.field("parent", &self.parent)
			.field("transform", &self.transform)
			.finish()
	}
}
impl Drop for Spatial {
	fn drop(&mut self) {
		SPATIAL_REGISTRY.remove(self);
	}
}

#[derive(Debug, Deref, Handler)]
pub struct SpatialRef {
	#[deref]
	data: Arc<Spatial>,
}
impl SpatialRefHandler for SpatialRef {}
interface!(SpatialInterface);
impl SpatialInterfaceHandler for SpatialInterface {
	async fn create_spatial(
		&self,
		_ctx: gluon::Context,
		parent: SpatialRefProxy,
		transform: Transform,
	) -> Result<CreatedSpatial, CreateError> {
		let parent = parent.owned().ok_or(CreateError::InvalidRef)?;
		let spatial = SpatialObject::new(Some(&parent.data), transform.to_mat4());
		Ok(CreatedSpatial {
			spatial_ref: spatial
				.local_handler::<SpatialObject>()
				.unwrap()
				.spatial_ref
				.clone(),
			spatial,
		})
	}

	async fn get_relative_bounding_box(
		&self,
		_ctx: gluon::Context,
		relative_to: SpatialRefProxy,
		spatial: SpatialRefProxy,
	) -> Result<BoundingBox, SpatialRefOpError> {
		let relative_to = relative_to
			.owned()
			.ok_or(SpatialRefOpError::RelativeToInvalid)?;
		let spatial = spatial
			.owned()
			.ok_or(SpatialRefOpError::SpatialRefInvalid)?;
		let mat = Spatial::space_to_space_matrix(Some(&spatial), Some(&relative_to));
		let bb = spatial.get_bounding_box();
		let bounds = Aabb::enclosing([
			mat.transform_point3(bb.min().into()),
			mat.transform_point3(bb.max().into()),
		])
		.unwrap();

		Ok(BoundingBox {
			center: Vec3::from(bounds.center).into(),
			extents: Vec3::from(bounds.half_extents * 2.0).into(),
		})
	}

	async fn get_relative_transform(
		&self,
		_ctx: gluon::Context,
		relative_to: SpatialRefProxy,
		spatial: SpatialRefProxy,
	) -> Result<Transform, SpatialRefOpError> {
		let relative_to = relative_to
			.owned()
			.ok_or(SpatialRefOpError::RelativeToInvalid)?;
		let spatial = spatial
			.owned()
			.ok_or(SpatialRefOpError::SpatialRefInvalid)?;
		let (scale, rotation, position) =
			Spatial::space_to_space_matrix(Some(&spatial), Some(&relative_to))
				.to_scale_rotation_translation();

		Ok(Transform {
			translation: position.into(),
			rotation: rotation.into(),
			scale: scale.into(),
		})
	}
}

impl_proxy!(SpatialProxy, SpatialObject);

#[cfg(test)]
mod moved_callback_tests {
	use super::*;
	use std::sync::atomic::{AtomicUsize, Ordering};

	fn counter() -> (Arc<AtomicUsize>, impl Fn() + Send + Sync + 'static) {
		let count = Arc::new(AtomicUsize::new(0));
		let cb = {
			let count = count.clone();
			move || {
				count.fetch_add(1, Ordering::Relaxed);
			}
		};
		(count, cb)
	}

	// The original bug: a callback registered on a child *after* it was parented must
	// still fire when the ancestor moves.
	#[test]
	fn callback_registered_after_parenting_fires_on_ancestor_move() {
		let parent = Spatial::test_new(None, Mat4::IDENTITY);
		let child = Spatial::test_new(None, Mat4::IDENTITY);
		child.set_spatial_parent(&parent).unwrap();

		let (count, cb) = counter();
		let _guard = child.moved_callback(cb);

		parent.set_local_transform(Mat4::from_translation(Vec3::X));
		assert_eq!(count.load(Ordering::Relaxed), 1);
	}

	// Propagation has to reach through multiple ancestor levels.
	#[test]
	fn callback_fires_for_grandparent_move() {
		let grandparent = Spatial::test_new(None, Mat4::IDENTITY);
		let parent = Spatial::test_new(None, Mat4::IDENTITY);
		let child = Spatial::test_new(None, Mat4::IDENTITY);
		parent.set_spatial_parent(&grandparent).unwrap();
		child.set_spatial_parent(&parent).unwrap();

		let (count, cb) = counter();
		let _guard = child.moved_callback(cb);

		grandparent.set_local_transform(Mat4::from_translation(Vec3::Y));
		assert_eq!(count.load(Ordering::Relaxed), 1);
	}

	// Reparenting must detach the callback from the old chain and attach it to the new.
	#[test]
	fn reparenting_moves_callback_between_chains() {
		let old_parent = Spatial::test_new(None, Mat4::IDENTITY);
		let new_parent = Spatial::test_new(None, Mat4::IDENTITY);
		let child = Spatial::test_new(None, Mat4::IDENTITY);
		child.set_spatial_parent(&old_parent).unwrap();

		let (count, cb) = counter();
		let _guard = child.moved_callback(cb);

		// The reparent itself changes the child's global pose → fires once.
		child.set_spatial_parent(&new_parent).unwrap();
		assert_eq!(count.load(Ordering::Relaxed), 1);

		// Old parent no longer drives the callback.
		old_parent.set_local_transform(Mat4::from_translation(Vec3::X));
		assert_eq!(count.load(Ordering::Relaxed), 1);

		// New parent does.
		new_parent.set_local_transform(Mat4::from_translation(Vec3::X));
		assert_eq!(count.load(Ordering::Relaxed), 2);
	}

	// A subtree's aggregated callbacks travel with it when an intermediate node moves.
	#[test]
	fn reparenting_subtree_carries_descendant_callbacks() {
		let root = Spatial::test_new(None, Mat4::IDENTITY);
		let parent = Spatial::test_new(None, Mat4::IDENTITY);
		let child = Spatial::test_new(None, Mat4::IDENTITY);
		child.set_spatial_parent(&parent).unwrap();

		// Register on the deepest node *before* `parent` joins `root`.
		let (count, cb) = counter();
		let _guard = child.moved_callback(cb);

		// The reparent moves the whole subtree → the descendant's callback fires once.
		parent.set_spatial_parent(&root).unwrap();
		assert_eq!(count.load(Ordering::Relaxed), 1);

		root.set_local_transform(Mat4::from_translation(Vec3::Z));
		assert_eq!(count.load(Ordering::Relaxed), 2);
	}

	// Moving a child must not fire callbacks that live on its ancestors.
	#[test]
	fn child_move_does_not_fire_ancestor_callback() {
		let parent = Spatial::test_new(None, Mat4::IDENTITY);
		let child = Spatial::test_new(None, Mat4::IDENTITY);
		child.set_spatial_parent(&parent).unwrap();

		let (count, cb) = counter();
		let _guard = parent.moved_callback(cb);

		child.set_local_transform(Mat4::from_translation(Vec3::X));
		assert_eq!(count.load(Ordering::Relaxed), 0);

		parent.set_local_transform(Mat4::from_translation(Vec3::X));
		assert_eq!(count.load(Ordering::Relaxed), 1);
	}

	// Reparenting changes the node's global pose, so it must fire the node's own
	// moved callbacks even though no local transform was set.
	#[test]
	fn reparenting_fires_moved_callback() {
		let new_parent = Spatial::test_new(None, Mat4::from_translation(Vec3::X * 100.0));
		let node = Spatial::test_new(None, Mat4::IDENTITY);

		let (count, cb) = counter();
		let _guard = node.moved_callback(cb);

		node.set_spatial_parent(&new_parent).unwrap();
		assert_eq!(count.load(Ordering::Relaxed), 1);
	}

	// Dropping the guard must stop the callback firing, including from ancestors.
	#[test]
	fn dropping_guard_detaches_from_ancestors() {
		let parent = Spatial::test_new(None, Mat4::IDENTITY);
		let child = Spatial::test_new(None, Mat4::IDENTITY);
		child.set_spatial_parent(&parent).unwrap();

		let (count, cb) = counter();
		let guard = child.moved_callback(cb);
		drop(guard);

		parent.set_local_transform(Mat4::from_translation(Vec3::X));
		assert_eq!(count.load(Ordering::Relaxed), 0);
	}
}
impl_proxy!(SpatialRefProxy, SpatialRef);

// Micro-benchmarks for the hot spatial operations. Run with:
// cargo test --profile benching -- --ignored bench_ --nocapture --test-threads=1
#[cfg(test)]
mod spatial_benches {
	use super::*;
	use crate::core::microbench::bench;

	/// A parent→child chain of `depth` nodes, each offset 1 m on X. Returned root-first
	/// so the caller keeps every node alive; last element is the deepest node.
	fn chain(depth: usize) -> Vec<Arc<Spatial>> {
		let mut nodes = vec![Spatial::test_new(None, Mat4::from_translation(Vec3::X))];
		for _ in 1..depth {
			let parent = nodes.last().unwrap().clone();
			nodes.push(Spatial::test_new(
				Some(parent),
				Mat4::from_translation(Vec3::X),
			));
		}
		nodes
	}

	#[test]
	#[ignore]
	fn bench_spatial_global_transform() {
		for depth in [1, 8, 32] {
			let nodes = chain(depth);
			let deepest = nodes.last().unwrap();
			bench(&format!("global_transform (depth {depth})"), || {
				deepest.global_transform()
			});
		}
	}

	#[test]
	#[ignore]
	fn bench_spatial_space_to_space_matrix() {
		// Two independent depth-8 branches, as in a typical queryable-vs-zone test.
		let from = chain(8);
		let to = chain(8);
		let (from, to) = (from.last().unwrap(), to.last().unwrap());
		bench("space_to_space_matrix (two depth-8 chains)", || {
			Spatial::space_to_space_matrix(Some(from), Some(to))
		});
	}

	#[test]
	#[ignore]
	fn bench_spatial_visible() {
		for depth in [1, 8, 32] {
			let nodes = chain(depth);
			let deepest = nodes.last().unwrap();
			bench(&format!("visible (depth {depth})"), || deepest.visible());
		}
	}

	#[test]
	#[ignore]
	fn bench_spatial_set_local_transform() {
		let node = Spatial::test_new(None, Mat4::IDENTITY);
		let mat = Mat4::from_translation(Vec3::X);
		bench("set_local_transform (no callbacks)", || {
			node.set_local_transform(mat)
		});

		// Callbacks registered on 64 descendants all aggregate onto the root, so this
		// is the cost of moving a node with a subtree of 64 watchers.
		let root = Spatial::test_new(None, Mat4::IDENTITY);
		let children: Vec<Arc<Spatial>> = (0..64)
			.map(|_| Spatial::test_new(Some(root.clone()), Mat4::IDENTITY))
			.collect();
		let _guards: Vec<MovedCallback> = children
			.iter()
			.map(|child| child.moved_callback(|| {}))
			.collect();
		bench("set_local_transform (64 descendant callbacks)", || {
			root.set_local_transform(mat)
		});
	}

	#[test]
	#[ignore]
	fn bench_spatial_reparent() {
		// Ping-pong a subtree with 16 watched descendants between two parents —
		// exercises the ancestor-chain callback rewiring plus the new callback fire.
		let parent_a = Spatial::test_new(None, Mat4::IDENTITY);
		let parent_b = Spatial::test_new(None, Mat4::from_translation(Vec3::X));
		let node = Spatial::test_new(None, Mat4::IDENTITY);
		node.set_spatial_parent(&parent_a).unwrap();
		let children: Vec<Arc<Spatial>> = (0..16)
			.map(|_| {
				let child = Spatial::test_new(None, Mat4::IDENTITY);
				child.set_spatial_parent(&node).unwrap();
				child
			})
			.collect();
		let _guards: Vec<MovedCallback> = children
			.iter()
			.map(|child| child.moved_callback(|| {}))
			.collect();

		let mut on_a = true;
		bench("set_spatial_parent (16 descendant callbacks)", || {
			let target = if on_a { &parent_b } else { &parent_a };
			on_a = !on_a;
			node.set_spatial_parent(target).unwrap();
		});
	}

	#[test]
	#[ignore]
	fn bench_spatial_get_bounding_box() {
		let root = Spatial::test_new(None, Mat4::IDENTITY);
		let children: Vec<Arc<Spatial>> = (0..16)
			.map(|i| {
				let child = Spatial::test_new(None, Mat4::from_translation(Vec3::X * i as f32));
				child.set_spatial_parent(&root).unwrap();
				child
			})
			.collect();
		let _guards: Vec<BoundingBoxCalc> = children
			.iter()
			.map(|child| child.custom_bounding_box(Aabb::default))
			.collect();
		bench("get_bounding_box (16 children)", || root.get_bounding_box());
	}
}

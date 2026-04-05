use crate::bevy_int::entity_handle::EntityHandle;
use crate::core::error::Result;
use crate::core::registry::Registry;
use crate::nodes::{ProxyExt, ref_owned};
use crate::{PION, impl_proxy, impl_transaction_handler, interface};
use bevy::ecs::entity::EntityHashMap;
use bevy::prelude::Transform as BevyTransform;
use bevy::prelude::*;
use bevy::render::primitives::Aabb;
use binderbinder::binder_object::BinderObject;
use glam::{Mat4, Quat};
use gluon_wire::GluonCtx;
use gluon_wire::drop_tracking::DropNotifier;
use parking_lot::Mutex;
use stardust_xr_protocol::protocol::spatial::{
	BoundingBox, PartialTransform, Spatial as SpatialProxy, SpatialHandler,
	SpatialInterfaceHandler, SpatialRef as SpatialRefProxy, SpatialRefHandler, Transform,
};
use stardust_xr_server_foundation::bail;
use std::fmt::Debug;
use std::sync::{Arc, Weak};
use std::{f32, ptr};
use tokio::sync::RwLock;

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
		.filter(|v| v.entity.blocking_read().is_none())
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

static SPATIAL_REGISTRY: Registry<SpatialInner> = Registry::new();

#[derive(Clone, Component, Debug)]
#[require(BevyTransform, Visibility)]
pub struct SpatialNode(pub Weak<SpatialInner>);

const EPSILON: f32 = 0.00001;

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

pub type BoundingBoxCalc = Arc<dyn Fn() -> Aabb + Send + Sync + 'static>;

#[derive(Debug, Deref)]
pub struct SpatialRef {
	#[deref]
	spatial: Arc<SpatialInner>,
	drop_notifs: RwLock<Vec<DropNotifier>>,
}
impl SpatialRefHandler for SpatialRef {
	async fn drop_notification_requested(&self, notifier: DropNotifier) {
		self.drop_notifs.write().await.push(notifier);
	}
}

pub type Spatial = BinderObject<SpatialInner>;
pub struct SpatialInner {
	self_ref: SelfRef<Spatia>,
	spatial_ref: Arc<BinderObject<SpatialRef>>,
	entity: RwLock<Option<EntityHandle>>,
	parent: RwLock<Option<Arc<Spatial>>>,
	transform: RwLock<Mat4>,
	children: Registry<Spatial>,
	bounding_box_calc: BoundingBoxCalc,
	drop_notifs: RwLock<Vec<DropNotifier>>,
}
impl Debug for SpatialInner {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("SpatialInner")
			.field("self_ref", &self.self_ref)
			.field("spatial_ref", &self.spatial_ref)
			.field("entity", &self.entity)
			.field("parent", &self.parent)
			.field("transform", &self.transform)
			.field("children", &self.children)
			.field("drop_notifs", &self.drop_notifs)
			.finish()
	}
}

impl Spatial {
	pub fn new(parent: Option<&Arc<Spatial>>, transform: Mat4) -> Arc<Self> {
		let spatial = PION.register_object_cyclic(|self_ref| {
			let spatial_ref = PION.register_object(SpatialRef {
				spatial: self_ref.clone(),
				drop_notifs: RwLock::default(),
			});
			ref_owned(&spatial_ref);

			SpatialInner {
				self_ref,
				spatial_ref: spatial_ref.clone(),
				entity: RwLock::new(None),
				parent: RwLock::new(parent.cloned()),
				transform: RwLock::new(transform),
				children: Registry::new(),
				bounding_box_calc: Registry::new(),
				drop_notifs: RwLock::default(),
			}
		});
		ref_owned(&spatial);
		SPATIAL_REGISTRY.add_raw(&data);
		spatial.mark_dirty();
		spatial
	}
	pub fn get_ref(&self) -> &Arc<BinderObject<SpatialRef>> {
		&self.spatial_ref
	}
}

impl SpatialInner {
	pub async fn custom_bounding_box(
		&self,
		calc: impl Fn() -> Aabb + Send + Sync + 'static,
	) -> BoundingBoxCalc {
		let arc: BoundingBoxCalc = Arc::new(calc);
		self.bounding_box_calc.add_raw(&arc);
		arc
	}
	pub async fn set_entity(&self, entity: EntityHandle) {
		self.entity.write().await.replace(entity);
		self.mark_dirty();
		for child in self.children.get_valid_contents() {
			child.mark_dirty();
		}
	}
	pub async fn get_entity(&self) -> Option<Entity> {
		self.entity.read().await.as_ref().map(|v| v.get())
	}

	pub fn space_to_space_matrix(from: Option<&SpatialInner>, to: Option<&SpatialInner>) -> Mat4 {
		let space_to_world_matrix = from.map_or(Mat4::IDENTITY, |from| from.global_transform());
		let world_to_space_matrix = to.map_or(Mat4::IDENTITY, |to| to.global_transform().inverse());
		world_to_space_matrix * space_to_world_matrix
	}

	// the output bounds are probably way bigger than they need to be
	pub async fn get_bounding_box(&self) -> Aabb {
		// let Some(node) = self.node() else {
		// 	return Aabb::default();
		// };
		// let mut bounds = match self.bounding_box_calc.get() {
		// 	Some(f) => f(&node).await,
		// 	None => Aabb::default(),
		// };
		let mut bounds = Aabb::default();
		for f in self.bounding_box_calc.get_valid_contents() {
			let b = f();
			bounds = Aabb::enclosing(
				[b.min(), b.max(), bounds.min(), bounds.max()]
					.into_iter()
					.map(Vec3::from),
			)
			.unwrap_or(bounds);
		}
		for child in self.children.get_valid_contents() {
			let mat = child.local_transform().await;
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
	pub(super) async fn mark_dirty(&self) {
		let Some(entity) = self.entity.read().await.as_ref().map(|v| v.get()) else {
			return;
		};
		let enabled = self.local_visible().await;
		let transform = if enabled {
			Some(BevyTransform::from_matrix(self.local_transform().await))
		} else {
			None
		};
		let parent = if let Some(v) = self.get_parent() {
			v.entity.read().await.as_ref().map(|v| v.get())
		} else {
			None
		};
		UPDATED_SPATIALS_NODES
			.lock()
			.insert(entity, (transform, parent));
	}

	pub async fn local_transform(&self) -> Mat4 {
		*self.transform.read().await
	}

	async fn local_visible(&self) -> bool {
		// Check our own scale by looking at matrix column lengths
		let mat = self.local_transform().await;
		let x_scale = mat.x_axis.length_squared();
		let y_scale = mat.y_axis.length_squared();
		let z_scale = mat.z_axis.length_squared();

		x_scale > EPSILON || y_scale > EPSILON || z_scale > EPSILON
	}
	/// Check if this node or any ancestor has zero scale (for visibility culling)
	pub async fn visible(&self) -> bool {
		// Check parent chain
		if let Some(parent) = self.get_parent()
			&& !parent.local_visible().await
		{
			return false;
		}

		// Check our own scale by looking at matrix column lengths
		self.local_visible().await
	}
	pub fn global_transform(&self) -> Mat4 {
		let parent_transform = self
			.get_parent()
			.as_deref()
			.map(Self::global_transform)
			.unwrap_or_default();
		parent_transform * self.local_transform()
	}
	pub async fn set_local_transform(&self, transform: Mat4) {
		*self.transform.write().await = transform;
		self.mark_dirty();
	}
	pub fn set_local_transform_components(
		&self,
		reference_space: Option<&SpatialInner>,
		transform: Transform,
	) {
		if reference_space == Some(self) {
			self.set_local_transform(transform.to_mat4(true, true, true) * self.local_transform());
			return;
		}
		let reference_to_parent_transform = reference_space
			.map(|reference_space| {
				SpatialMut::space_to_space_matrix(
					Some(reference_space),
					self.get_parent().as_deref(),
				)
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

	pub fn is_ancestor_of(&self, spatial: Arc<SpatialMut>) -> bool {
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

	async fn get_parent(&self) -> Option<Arc<SpatialInner>> {
		self.parent.read().await.clone()
	}
	async fn set_parent(self: &Arc<Self>, new_parent: &Arc<SpatialInner>) {
		if let Some(parent) = self.get_parent() {
			parent.children.remove(self);
		}
		new_parent.children.add_raw(self);

		*self.parent.write().await = Some(new_parent.clone());
		self.mark_dirty();
	}

	pub fn set_spatial_parent(self: &Arc<Self>, parent: &Arc<SpatialInner>) -> Result<()> {
		if self.is_ancestor_of(parent.clone()) {
			bail!("Setting spatial parent would cause a loop");
		}
		self.set_parent(parent);

		Ok(())
	}
	pub fn set_spatial_parent_in_place(self: &Arc<Self>, parent: &Arc<SpatialMut>) -> Result<()> {
		if self.is_ancestor_of(parent.clone()) {
			bail!("Setting spatial parent would cause a loop");
		}

		self.set_local_transform(SpatialMut::space_to_space_matrix(Some(self), Some(parent)));
		self.set_parent(parent);

		Ok(())
	}
}
static UPDATED_SPATIALS_NODES: Mutex<EntityHashMap<(Option<BevyTransform>, Option<Entity>)>> =
	Mutex::new(EntityHashMap::new());
impl SpatialHandler for SpatialInner {
	async fn spatial_ref(&self, _ctx: GluonCtx) -> SpatialRefProxy {
		SpatialRefProxy::from_handler(&self.spatial_ref)
	}

	async fn get_local_bounding_box(&self, _ctx: GluonCtx) -> BoundingBox {
		let bounds = self.get_bounding_box().await;
		BoundingBox {
			center: bounds.center.into(),
			extents: (bounds.half_extents * 2.0).into(),
		}
	}

	async fn get_relative_bounding_box(
		&self,
		_ctx: GluonCtx,
		relative_to: SpatialRefProxy,
	) -> BoundingBox {
		let Some(relative_to) = relative_to.owned() else {
			// TODO: return error instead
			panic!("Invalid SpatialRef used");
			// return;
		};
		let mat = SpatialInner::space_to_space_matrix(Some(self), Some(&relative_to));
		let bb = self.get_bounding_box().await;
		let bounds = Aabb::enclosing([
			mat.transform_point3(bb.min().into()),
			mat.transform_point3(bb.max().into()),
		])
		.unwrap();

		BoundingBox {
			center: Vec3::from(bounds.center).into(),
			extents: Vec3::from(bounds.half_extents * 2.0).into(),
		}
	}

	async fn get_relative_transform(
		&self,
		_ctx: GluonCtx,
		relative_to: SpatialRefProxy,
	) -> Transform {
		let Some(relative_to) = relative_to.owned() else {
			// TODO: return error instead
			panic!("Invalid SpatialRef used");
			// return;
		};
		let (scale, rotation, position) =
			SpatialInner::space_to_space_matrix(Some(self), Some(&relative_to))
				.to_scale_rotation_translation();

		Transform {
			translation: position.into(),
			rotation: rotation.into(),
			// TODO: actually just store pos rot and a single scale float
			scale: scale.max_element(),
		}
	}

	fn set_parent(&self, _ctx: GluonCtx, parent: SpatialRefProxy) {
		let Some(parent) = parent.owned() else {
			error!("Invalid SpatialRef used as parent");
			return;
		};
		self.set_spatial_parent(&parent)?;
	}

	fn set_parent_in_place(&self, _ctx: GluonCtx, parent: SpatialRefProxy) {
		let Some(parent) = parent.owned() else {
			error!("Invalid SpatialRef used as parent");
			return;
		};
		self.set_spatial_parent_in_place(parent);
	}

	fn set_local_transform(&self, _ctx: GluonCtx, transform: PartialTransform) {
		self.set_local_transform_components(None, transform);
	}

	fn set_relative_transform(
		&self,
		_ctx: GluonCtx,
		relative_to: SpatialRefProxy,
		transform: PartialTransform,
	) {
		let Some(relative_to) = relative_to.owned() else {
			error!("Invalid SpatialRef used");
			return;
		};
		self.set_local_transform_components(Some(&relative_to), transform);
	}

	async fn drop_notification_requested(&self, notifier: DropNotifier) {
		self.drop_notifs.write().await.push(notifier);
	}
}
impl Debug for SpatialMut {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Spatial")
			.field("parent", &self.parent)
			.field("transform", &self.transform)
			.finish()
	}
}
impl Drop for SpatialInner {
	fn drop(&mut self) {
		SPATIAL_REGISTRY.remove(self);
	}
}

interface!(SpatialInterface);
impl SpatialInterfaceHandler for SpatialInterface {
	async fn create_spatial(
		&self,
		_ctx: GluonCtx,
		parent: SpatialRefProxy,
		transform: Transform,
	) -> SpatialProxy {
		// SpatialMut::new(parent, transform);
		todo!()
	}

	async fn get_relative_bounding_box(
		&self,
		_ctx: GluonCtx,
		relative_to: SpatialRefProxy,
		spatial: SpatialRefProxy,
	) -> BoundingBox {
		let Some(relative_to) = relative_to.owned() else {
			// TODO: return error instead
			panic!("Invalid SpatialRef used");
			// return;
		};
		let Some(spatial) = spatial.owned() else {
			// TODO: return error instead
			panic!("Invalid SpatialRef used");
			// return;
		};
		let mat = SpatialInner::space_to_space_matrix(Some(&spatial), Some(&relative_to));
		let bb = spatial.get_bounding_box().await;
		let bounds = Aabb::enclosing([
			mat.transform_point3(bb.min().into()),
			mat.transform_point3(bb.max().into()),
		])
		.unwrap();

		BoundingBox {
			center: Vec3::from(bounds.center).into(),
			extents: Vec3::from(bounds.half_extents * 2.0).into(),
		}
	}

	async fn get_relative_transform(
		&self,
		_ctx: GluonCtx,
		relative_to: SpatialRefProxy,
		spatial: SpatialRefProxy,
	) -> Transform {
		let Some(relative_to) = relative_to.owned() else {
			// TODO: return error instead
			panic!("Invalid SpatialRef used");
			// return;
		};
		let Some(spatial) = spatial.owned() else {
			// TODO: return error instead
			panic!("Invalid SpatialRef used");
			// return;
		};
		let (scale, rotation, position) =
			SpatialInner::space_to_space_matrix(Some(&spatial), Some(&relative_to))
				.to_scale_rotation_translation();

		Transform {
			translation: position.into(),
			rotation: rotation.into(),
			// TODO: actually just store pos rot and a single scale float
			scale: scale.max_element(),
		}
	}

	async fn drop_notification_requested(&self, notifier: DropNotifier) {
		self.drop_notifs.write().await.push(notifier);
	}
}

impl_proxy!(SpatialProxy, SpatialInner);
impl_proxy!(SpatialRefProxy, SpatialRef);
impl_transaction_handler!(SpatialInner);
impl_transaction_handler!(SpatialRef);

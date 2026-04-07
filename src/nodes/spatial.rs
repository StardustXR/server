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
use stardust_xr_protocol::spatial::{
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

#[derive(Clone, Component, Debug)]
#[require(BevyTransform, Visibility)]
pub struct SpatialNode(pub Weak<Spatial>);

const EPSILON: f32 = 0.00001;

pub trait TransformExt {
	fn to_mat4(&self) -> Mat4;
}
impl TransformExt for Transform {
	fn to_mat4(&self) -> Mat4 {
		// Zero scale values break everything
		let scale = if self.scale == 0.0 {
			EPSILON
		} else {
			self.scale
		};

		Mat4::from_scale_rotation_translation(
			Vec3::splat(scale),
			self.rotation.mint(),
			self.translation.mint(),
		)
	}
}
impl TransformExt for PartialTransform {
	fn to_mat4(&self) -> Mat4 {
		// Zero scale values break everything
		let scale = if self.scale.unwrap_or(1.0) == 0.0 {
			EPSILON
		} else {
			self.scale.unwrap_or(1.0)
		};

		Mat4::from_scale_rotation_translation(
			Vec3::splat(scale),
			self.rotation.map(|v| v.mint()).unwrap_or(Quat::IDENTITY),
			self.translation.map(|v| v.mint()).unwrap_or(Vec3::ZERO),
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

pub struct Spatial {
	entity: Mutex<Option<EntityHandle>>,
	parent: Mutex<Option<Arc<Spatial>>>,
	transform: Mutex<Mat4>,
	children: Registry<Spatial>,
	bounding_box_calc: Registry<dyn Fn() -> Aabb + Send + Sync + 'static>,
}
impl Debug for Spatial {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Spatial")
			.field("parent", &self.parent)
			.field("transform", &self.transform)
			.field("children", &self.children)
			.finish()
	}
}

#[derive(Deref)]
pub struct SpatialObject {
	#[deref]
	data: Arc<Spatial>,
	spatial_ref: Arc<BinderObject<SpatialRef>>,
	drop_notifs: RwLock<Vec<DropNotifier>>,
}
impl SpatialObject {
	pub fn new(parent: Option<&Arc<Spatial>>, transform: Mat4) -> Arc<BinderObject<Self>> {
		let data = Arc::new(Spatial {
			entity: Mutex::new(None),
			parent: Mutex::new(parent.cloned()),
			transform: Mutex::new(transform),
			children: Registry::new(),
			bounding_box_calc: Registry::new(),
		});
		SPATIAL_REGISTRY.add_raw(&data);
		let spatial_ref = PION.register_object(SpatialRef {
			data: data.clone(),
			drop_notifs: RwLock::default(),
		});
		ref_owned(&spatial_ref);
		let spatial = PION.register_object(SpatialObject {
			drop_notifs: RwLock::default(),
			data,
			spatial_ref,
		});
		ref_owned(&spatial);
		spatial.mark_dirty();
		spatial
	}
	pub fn get_ref(&self) -> &Arc<BinderObject<SpatialRef>> {
		&self.spatial_ref
	}
}

impl Spatial {
	pub fn custom_bounding_box(
		&self,
		calc: impl Fn() -> Aabb + Send + Sync + 'static,
	) -> BoundingBoxCalc {
		let arc = BoundingBoxCalc(Arc::new(calc));
		self.bounding_box_calc.add_raw(&arc.0);
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
			let mat = child.local_transform();
			let child_aabb = Box::pin(child.get_bounding_box());
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
		let Some(entity) = self.entity.lock().as_ref().map(|v| v.get()) else {
			return;
		};
		let enabled = self.local_visible();
		let transform = if enabled {
			Some(BevyTransform::from_matrix(self.local_transform()))
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

	fn local_visible(&self) -> bool {
		// Check our own scale by looking at matrix column lengths
		let mat = self.local_transform();
		let x_scale = mat.x_axis.length_squared();
		let y_scale = mat.y_axis.length_squared();
		let z_scale = mat.z_axis.length_squared();

		x_scale > EPSILON || y_scale > EPSILON || z_scale > EPSILON
	}
	/// Check if this node or any ancestor has zero scale (for visibility culling)
	pub fn visible(&self) -> bool {
		// Check parent chain
		if let Some(parent) = self.get_parent()
			&& !parent.local_visible()
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
		*self.transform.lock() = transform;
		self.mark_dirty();
	}
	pub fn set_local_transform_components(
		&self,
		reference_space: Option<&Spatial>,
		transform: PartialTransform,
	) {
		if reference_space.is_some_and(|reference| reference as *const _ == self as *const _) {
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
			reference_space_pos = pos.mint()
		}
		if let Some(rot) = transform.rotation {
			reference_space_rot = rot.mint()
		} else if reference_space_rot.is_nan() {
			reference_space_rot = Quat::IDENTITY;
		}
		if let Some(scl) = transform.scale {
			reference_space_scl = Vec3::splat(scl)
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
		if let Some(parent) = self.get_parent() {
			parent.children.remove(self);
		}
		new_parent.children.add_raw(self);

		*self.parent.lock() = Some(new_parent.clone());
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
	async fn spatial_ref(&self, _ctx: GluonCtx) -> SpatialRefProxy {
		SpatialRefProxy::from_handler(&self.spatial_ref)
	}

	async fn get_local_bounding_box(&self, _ctx: GluonCtx) -> BoundingBox {
		let bounds = self.get_bounding_box();
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
		let mat = Spatial::space_to_space_matrix(Some(self), Some(&relative_to));
		let bb = self.get_bounding_box();
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
			Spatial::space_to_space_matrix(Some(self), Some(&relative_to))
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
		_ = self
			.set_spatial_parent(&parent)
			.inspect_err(|err| error!("error while setting spatial parent: {err}"));
	}

	fn set_parent_in_place(&self, _ctx: GluonCtx, parent: SpatialRefProxy) {
		let Some(parent) = parent.owned() else {
			error!("Invalid SpatialRef used as parent");
			return;
		};
		_ = self
			.set_spatial_parent_in_place(&parent.data)
			.inspect_err(|err| error!("error while setting spatial parent in place: {err}"));
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

#[derive(Debug, Deref)]
pub struct SpatialRef {
	#[deref]
	data: Arc<Spatial>,
	drop_notifs: RwLock<Vec<DropNotifier>>,
}
impl SpatialRefHandler for SpatialRef {
	async fn drop_notification_requested(&self, notifier: DropNotifier) {
		self.drop_notifs.write().await.push(notifier);
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
		let Some(parent) = parent.owned() else {
			// TODO: return error instead
			panic!("Invalid SpatialRef used");
			// return;
		};
		SpatialProxy::from_handler(&SpatialObject::new(Some(&parent.data), transform.to_mat4()))
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
		let mat = Spatial::space_to_space_matrix(Some(&spatial), Some(&relative_to));
		let bb = spatial.get_bounding_box();
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
			Spatial::space_to_space_matrix(Some(&spatial), Some(&relative_to))
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

impl_proxy!(SpatialProxy, SpatialObject);
impl_proxy!(SpatialRefProxy, SpatialRef);
impl_transaction_handler!(SpatialObject);
impl_transaction_handler!(SpatialRef);

pub mod zone;

use self::zone::Zone;
use super::alias::Alias;
use super::fields::{Field, FieldTrait};
use super::{Aspect, AspectIdentifier};
use crate::bail;
use crate::core::client::Client;
use crate::core::error::Result;
use crate::core::registry::Registry;
use crate::nodes::{Node, OWNED_ASPECT_ALIAS_INFO};
use bevy::prelude::Transform as BevyTransform;
use bevy::prelude::*;
use bevy::render::primitives::Aabb;
use color_eyre::eyre::OptionExt;
use glam::{Mat4, Quat, Vec3, vec3a};
use mint::Vector3;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use std::fmt::Debug;
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock, Weak};
use std::{f32, ptr};

pub struct SpatialNodePlugin;
impl Plugin for SpatialNodePlugin {
	fn build(&self, app: &mut App) {
		app.add_systems(
			PostUpdate,
			update_spatial_nodes.before(TransformSystem::TransformPropagate),
		);
	}
}

fn update_spatial_nodes(
	mut query: Query<(
		&mut BevyTransform,
		&SpatialNode,
		Option<&ChildOf>,
		&mut Visibility,
	)>,
	parent_query: Query<&GlobalTransform>,
) {
	query
		.par_iter_mut()
		.for_each(|(mut transform, spatial_node, child_of, mut vis)| {
			let _span = debug_span!("updating spatial node").entered();
			let Some(spatial) = spatial_node.0.upgrade() else {
				return;
			};
			if spatial
				.node()
				.is_some_and(|v| !v.enabled.load(Ordering::Relaxed))
			{
				if !matches!(*vis, Visibility::Hidden) {
					*vis = Visibility::Hidden;
				}
				return;
			}
			let mat4 =
				debug_span!("getting global transform").in_scope(|| spatial.global_transform());
			let (scale, _, _) = mat4.to_scale_rotation_translation();
			match (*vis, scale == Vec3::ZERO) {
				(Visibility::Inherited | Visibility::Visible, true) => {
					*vis = Visibility::Hidden;
				}
				(Visibility::Hidden, false) => {
					*vis = Visibility::Inherited;
				}
				_ => {}
			}
			match child_of {
				Some(child_of) => {
					let Ok(parent) = parent_query.get(child_of.0) else {
						warn!("SpatialNode bevy Parent doesn't have global transform");
						return;
					};
					*transform =
						BevyTransform::from_matrix(parent.compute_matrix().inverse() * mat4);
				}
				None => {
					*transform = BevyTransform::from_matrix(mat4);
				}
			}
		});
}

#[derive(Clone, Component, Debug)]
#[require(BevyTransform, Visibility)]
pub struct SpatialNode(pub Weak<Spatial>);

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
		let scale = scale
			.then_some(self.scale)
			.flatten()
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
	pub static ref EXPORTED_SPATIALS: Mutex<FxHashMap<u64, Arc<Node>>> = Mutex::new(FxHashMap::default());
}

static ZONEABLE_REGISTRY: Registry<Spatial> = Registry::new();

pub struct Spatial {
	pub node: Weak<Node>,
	parent: Mutex<Option<Arc<Spatial>>>,
	old_parent: Mutex<Option<Arc<Spatial>>>,
	transform: Mutex<Mat4>,
	zone: Mutex<Weak<Zone>>,
	children: Registry<Spatial>,
	pub bounding_box_calc: OnceLock<fn(&Node) -> Aabb>,
}

impl Spatial {
	pub fn new(node: Weak<Node>, parent: Option<Arc<Spatial>>, transform: Mat4) -> Arc<Self> {
		Arc::new(Spatial {
			node,
			parent: Mutex::new(parent),
			old_parent: Mutex::new(None),
			transform: Mutex::new(transform),
			zone: Mutex::new(Weak::new()),
			children: Registry::new(),
			bounding_box_calc: OnceLock::default(),
		})
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
	pub fn get_bounding_box(&self) -> Aabb {
		let Some(node) = self.node() else {
			return Aabb::default();
		};
		let mut bounds = self
			.bounding_box_calc
			.get()
			.map(|b| (b)(&node))
			.unwrap_or_default();
		for child in self.children.get_valid_contents() {
			let mat = child.local_transform();
			let child_aabb = child.get_bounding_box();
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

	pub fn local_transform(&self) -> Mat4 {
		*self.transform.lock()
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
	}
	pub fn set_local_transform_components(
		&self,
		reference_space: Option<&Spatial>,
		transform: Transform,
	) {
		if reference_space == Some(self) {
			self.set_local_transform(
				parse_transform(transform, true, true, true) * self.local_transform(),
			);
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
		self.parent.lock().clone()
	}
	fn set_parent(self: &Arc<Self>, new_parent: Option<&Arc<Spatial>>) {
		if let Some(parent) = self.get_parent() {
			parent.children.remove(self);
		}
		if let Some(new_parent) = &new_parent {
			new_parent.children.add_raw(self);
		}

		*self.parent.lock() = new_parent.cloned();
	}

	pub fn set_spatial_parent(self: &Arc<Self>, parent: Option<&Arc<Spatial>>) -> Result<()> {
		let is_ancestor = parent
			.as_ref()
			.map(|parent| self.is_ancestor_of((*parent).clone()))
			.unwrap_or(false);
		if is_ancestor {
			bail!("Setting spatial parent would cause a loop");
		}
		self.set_parent(parent);

		Ok(())
	}
	pub fn set_spatial_parent_in_place(
		self: &Arc<Self>,
		parent: Option<&Arc<Spatial>>,
	) -> Result<()> {
		let is_ancestor = parent
			.as_ref()
			.map(|parent| self.is_ancestor_of((*parent).clone()))
			.unwrap_or(false);
		if is_ancestor {
			bail!("Setting spatial parent would cause a loop");
		}

		self.set_local_transform(Spatial::space_to_space_matrix(
			Some(self),
			parent.map(AsRef::as_ref),
		));
		self.set_parent(parent);

		Ok(())
	}

	pub(self) fn zone_distance(&self) -> f32 {
		self.zone
			.lock()
			.upgrade()
			.map(|zone| zone.field.clone())
			.map(|field| field.distance(self, vec3a(0.0, 0.0, 0.0)))
			.unwrap_or(f32::NEG_INFINITY)
	}
}
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

		this_spatial.set_spatial_parent(Some(&parent))?;
		Ok(())
	}

	fn set_spatial_parent_in_place(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		parent: Arc<Node>,
	) -> Result<()> {
		let this_spatial = node.get_aspect::<Spatial>()?;
		let parent = parent.get_aspect::<Spatial>()?;

		this_spatial.set_spatial_parent_in_place(Some(&parent))?;
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
		EXPORTED_SPATIALS.lock().insert(id, node);
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
		let bounds = this_spatial.get_bounding_box();

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
		let bb = this_spatial.get_bounding_box();
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

pub fn parse_transform(transform: Transform, position: bool, rotation: bool, scale: bool) -> Mat4 {
	let position = position
		.then_some(transform.translation)
		.flatten()
		.unwrap_or_else(|| Vector3::from([0.0; 3]));
	let rotation = rotation
		.then_some(transform.rotation)
		.flatten()
		.unwrap_or_else(|| Quat::IDENTITY.into());
	let scale = scale
		.then_some(transform.scale)
		.flatten()
		.unwrap_or_else(|| Vector3::from([1.0; 3]));

	Mat4::from_scale_rotation_translation(scale.into(), rotation.into(), position.into())
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
		let transform = parse_transform(transform, true, true, true);
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
		let transform = parse_transform(transform, true, true, false);
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
			.map(|s| {
				Alias::create(
					s,
					&calling_client,
					SPATIAL_REF_ASPECT_ALIAS_INFO.clone(),
					None,
				)
				.unwrap()
			})
			.ok_or_eyre("Couldn't find spatial with that ID")?)
	}
}

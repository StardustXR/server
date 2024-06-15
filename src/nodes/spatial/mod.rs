pub mod zone;

use self::zone::Zone;
use super::fields::Field;
use super::{Aspect, Node};
use crate::core::client::Client;
use crate::core::registry::Registry;
use crate::create_interface;
use crate::nodes::OWNED_ASPECT_ALIAS_INFO;
use color_eyre::eyre::{eyre, Result};
use glam::{vec3a, Mat4, Quat, Vec3};
use mint::Vector3;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use std::fmt::Debug;
use std::ptr;
use std::sync::{Arc, Weak};
use stereokit_rust::maths::Bounds;

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

static ZONEABLE_REGISTRY: Registry<Spatial> = Registry::new();

pub struct Spatial {
	pub node: Weak<Node>,
	parent: Mutex<Option<Arc<Spatial>>>,
	old_parent: Mutex<Option<Arc<Spatial>>>,
	transform: Mutex<Mat4>,
	zone: Mutex<Weak<Zone>>,
	children: Registry<Spatial>,
	pub bounding_box_calc: OnceCell<fn(&Node) -> Bounds>,
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
			bounding_box_calc: OnceCell::default(),
		})
	}
	pub fn add_to(
		node: &Arc<Node>,
		parent: Option<Arc<Spatial>>,
		transform: Mat4,
		zoneable: bool,
	) -> Arc<Spatial> {
		let spatial = Spatial::new(Arc::downgrade(node), parent.clone(), transform);
		<Spatial as SpatialAspect>::add_node_members(node);
		if zoneable {
			ZONEABLE_REGISTRY.add_raw(&spatial);
		}
		if let Some(parent) = parent {
			parent.children.add_raw(&spatial);
		}
		<Spatial as SpatialRefAspect>::add_node_members(node);
		<Spatial as SpatialAspect>::add_node_members(node);
		node.add_aspect_raw(spatial.clone());
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
	pub fn get_bounding_box(&self) -> Bounds {
		let Some(node) = self.node() else {
			return Bounds::default();
		};
		let mut bounds = self
			.bounding_box_calc
			.get()
			.map(|b| (b)(&node))
			.unwrap_or_default();
		for child in self.children.get_valid_contents() {
			bounds.grown_box(child.get_bounding_box(), child.local_transform());
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
			return Err(eyre!("Setting spatial parent would cause a loop"));
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
			return Err(eyre!("Setting spatial parent would cause a loop"));
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
			.unwrap_or(f32::MAX)
	}
}
impl Aspect for Spatial {
	const NAME: &'static str = "Spatial";
}
impl SpatialRefAspect for Spatial {
	async fn get_local_bounding_box(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
	) -> Result<BoundingBox> {
		let this_spatial = node.get_aspect::<Spatial>()?;
		let bounds = this_spatial.get_bounding_box();

		Ok(BoundingBox {
			center: Vec3::from(bounds.center).into(),
			size: Vec3::from(bounds.dimensions).into(),
		})
	}

	async fn get_relative_bounding_box(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		relative_to: Arc<Node>,
	) -> Result<BoundingBox> {
		let this_spatial = node.get_aspect::<Spatial>()?;
		let relative_spatial = relative_to.get_aspect::<Spatial>()?;
		let center = Spatial::space_to_space_matrix(Some(&this_spatial), Some(&relative_spatial))
			.transform_point3([0.0; 3].into());
		let mut bounds = Bounds {
			center: center.into(),
			dimensions: [0.0; 3].into(),
		};
		bounds.grown_box(
			this_spatial.get_bounding_box(),
			Spatial::space_to_space_matrix(Some(&this_spatial), Some(&relative_spatial)),
		);

		Ok(BoundingBox {
			center: Vec3::from(bounds.center).into(),
			size: Vec3::from(bounds.dimensions).into(),
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

pub struct SpatialInterface;
impl InterfaceAspect for SpatialInterface {
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
}

create_interface!(SpatialInterface);

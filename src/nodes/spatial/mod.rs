pub mod zone;

use self::zone::Zone;
use super::fields::get_field;
use super::Node;
use crate::core::client::Client;
use crate::core::registry::Registry;
use crate::create_interface;
use color_eyre::eyre::{ensure, eyre, Result};
use glam::{vec3a, Mat4, Quat};
use mint::Vector3;
use nanoid::nanoid;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use std::fmt::Debug;
use std::os::fd::OwnedFd;
use std::ptr;
use std::sync::{Arc, Weak};
use stereokit::{bounds_grow_to_fit_box, Bounds};

stardust_xr_server_codegen::codegen_spatial_protocol!();
impl Transform {
	pub fn to_mat4(self, position: bool, rotation: bool, scale: bool) -> Mat4 {
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
	uid: String,
	pub(super) node: Weak<Node>,
	self_ref: Weak<Spatial>,
	parent: Mutex<Option<Arc<Spatial>>>,
	old_parent: Mutex<Option<Arc<Spatial>>>,
	pub(super) transform: Mutex<Mat4>,
	zone: Mutex<Weak<Zone>>,
	children: Registry<Spatial>,
	pub(super) bounding_box_calc: OnceCell<fn(&Node) -> Bounds>,
}

impl Spatial {
	pub fn new(node: Weak<Node>, parent: Option<Arc<Spatial>>, transform: Mat4) -> Arc<Self> {
		Arc::new_cyclic(|self_ref| Spatial {
			uid: nanoid!(),
			node,
			self_ref: self_ref.clone(),
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
	) -> Result<Arc<Spatial>> {
		ensure!(
			node.spatial.get().is_none(),
			"Internal: Node already has a Spatial aspect!"
		);
		let spatial = Spatial::new(Arc::downgrade(node), parent.clone(), transform);
		<Spatial as SpatialAspect>::add_node_members(node);
		if zoneable {
			ZONEABLE_REGISTRY.add_raw(&spatial);
		}
		if let Some(parent) = parent {
			parent.children.add_raw(&spatial);
		}
		let _ = node.spatial.set(spatial.clone());
		Ok(spatial)
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
			bounds = bounds_grow_to_fit_box(
				bounds,
				child.get_bounding_box(),
				Some(child.local_transform()),
			);
		}
		bounds
	}

	pub fn local_transform(&self) -> Mat4 {
		*self.transform.lock()
	}
	pub fn global_transform(&self) -> Mat4 {
		match self.get_parent() {
			Some(parent) => parent.global_transform() * *self.transform.lock(),
			None => *self.transform.lock(),
		}
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
	fn set_parent(&self, new_parent: Option<Arc<Spatial>>) {
		if let Some(parent) = self.get_parent() {
			parent.children.remove(self);
		}
		if let Some(new_parent) = &new_parent {
			new_parent
				.children
				.add_raw(&self.self_ref.upgrade().unwrap());
		}

		*self.parent.lock() = new_parent;
	}

	pub fn set_spatial_parent(&self, parent: Option<Arc<Spatial>>) -> Result<()> {
		let is_ancestor = parent
			.as_ref()
			.map(|parent| self.is_ancestor_of(parent.clone()))
			.unwrap_or(false);
		if is_ancestor {
			return Err(eyre!("Setting spatial parent would cause a loop"));
		}
		self.set_parent(parent);

		Ok(())
	}
	pub fn set_spatial_parent_in_place(&self, parent: Option<Arc<Spatial>>) -> Result<()> {
		let is_ancestor = parent
			.as_ref()
			.map(|parent| self.is_ancestor_of(parent.clone()))
			.unwrap_or(false);
		if is_ancestor {
			return Err(eyre!("Setting spatial parent would cause a loop"));
		}

		self.set_local_transform(Spatial::space_to_space_matrix(
			Some(self),
			parent.as_deref(),
		));
		self.set_parent(parent);

		Ok(())
	}

	pub(self) fn zone_distance(&self) -> f32 {
		self.zone
			.lock()
			.upgrade()
			.and_then(|zone| zone.field.upgrade())
			.map(|field| field.distance(self, vec3a(0.0, 0.0, 0.0)))
			.unwrap_or(f32::MAX)
	}
}
impl SpatialAspect for Spatial {
	fn get_local_bounding_box(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
	) -> impl std::future::Future<Output = Result<(BoundingBox, Vec<OwnedFd>)>> + Send + 'static {
		async move {
			let this_spatial = node
				.spatial
				.get()
				.ok_or_else(|| eyre!("Node doesn't have a spatial?"))?;
			let bounds = this_spatial.get_bounding_box();

			let return_value = BoundingBox {
				center: mint::Vector3::from(bounds.center),
				size: mint::Vector3::from(bounds.dimensions),
			};
			Ok((return_value, Vec::new()))
		}
	}

	fn get_relative_bounding_box(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		relative_to: Arc<Node>,
	) -> impl std::future::Future<Output = Result<(BoundingBox, Vec<OwnedFd>)>> + Send + 'static {
		async move {
			let this_spatial = node
				.spatial
				.get()
				.ok_or_else(|| eyre!("Node doesn't have a spatial?"))?;
			let relative_spatial = get_spatial(&relative_to, "Relative node")?;
			let center =
				Spatial::space_to_space_matrix(Some(&this_spatial), Some(&relative_spatial))
					.transform_point3([0.0; 3].into());
			let bounds = Bounds {
				center,
				dimensions: [0.0; 3].into(),
			};
			bounds_grow_to_fit_box(
				bounds,
				this_spatial.get_bounding_box(),
				Some(Spatial::space_to_space_matrix(
					Some(&this_spatial),
					Some(&relative_spatial),
				)),
			);

			let return_value = BoundingBox {
				center: mint::Vector3::from(bounds.center),
				size: mint::Vector3::from(bounds.dimensions),
			};
			Ok((return_value, Vec::new()))
		}
	}

	fn get_transform(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		relative_to: Arc<Node>,
	) -> impl std::future::Future<Output = Result<(Transform, Vec<OwnedFd>)>> + Send + 'static {
		async move {
			let this_spatial = node
				.spatial
				.get()
				.ok_or_else(|| eyre!("Node doesn't have a spatial?"))?;
			let relative_spatial = get_spatial(&relative_to, "Relative node")?;

			let (scale, rotation, position) = Spatial::space_to_space_matrix(
				Some(this_spatial.as_ref()),
				Some(relative_spatial.as_ref()),
			)
			.to_scale_rotation_translation();

			let return_value = Transform {
				translation: Some(position.into()),
				rotation: Some(rotation.into()),
				scale: Some(scale.into()),
			};
			Ok((return_value, Vec::new()))
		}
	}

	fn set_local_transform(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		transform: Transform,
	) -> Result<()> {
		let this_spatial = node
			.spatial
			.get()
			.ok_or_else(|| eyre!("Node doesn't have a spatial?"))?;
		this_spatial.set_local_transform_components(None, transform);
		Ok(())
	}
	fn set_relative_transform(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		relative_to: Arc<Node>,
		transform: Transform,
	) -> Result<()> {
		let this_spatial = node
			.spatial
			.get()
			.ok_or_else(|| eyre!("Node doesn't have a spatial?"))?;
		let relative_spatial = get_spatial(&relative_to, "Relative node")?;

		this_spatial.set_local_transform_components(Some(&relative_spatial), transform);
		Ok(())
	}

	fn set_spatial_parent(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		parent: Arc<Node>,
	) -> Result<()> {
		let this_spatial = node
			.spatial
			.get()
			.ok_or_else(|| eyre!("Node doesn't have a spatial?"))?;
		let parent = get_spatial(&parent, "Parent")?;

		this_spatial.set_spatial_parent(Some(parent))?;
		Ok(())
	}

	fn set_spatial_parent_in_place(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		parent: Arc<Node>,
	) -> Result<()> {
		let this_spatial = node
			.spatial
			.get()
			.ok_or_else(|| eyre!("Node doesn't have a spatial?"))?;
		let parent = get_spatial(&parent, "Parent")?;

		this_spatial.set_spatial_parent_in_place(Some(parent))?;
		Ok(())
	}

	fn set_zoneable(node: Arc<Node>, _calling_client: Arc<Client>, zoneable: bool) -> Result<()> {
		let spatial = node.spatial.get().unwrap();
		if zoneable {
			ZONEABLE_REGISTRY.add_raw(spatial);
		} else {
			ZONEABLE_REGISTRY.remove(spatial);
			zone::release(spatial);
		}
		Ok(())
	}
}
impl PartialEq for Spatial {
	fn eq(&self, other: &Self) -> bool {
		self.uid == other.uid
	}
}
impl Debug for Spatial {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Spatial")
			.field("uid", &self.uid)
			.field("parent", &self.parent)
			.field("old_parent", &self.old_parent)
			.field("transform", &self.transform)
			.finish()
	}
}
impl Drop for Spatial {
	fn drop(&mut self) {
		ZONEABLE_REGISTRY.remove(self);
		zone::release(self);
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

pub fn find_spatial(
	calling_client: &Arc<Client>,
	node_name: &'static str,
	node_path: &str,
) -> Result<Arc<Spatial>> {
	calling_client
		.get_node(node_name, node_path)?
		.get_aspect(node_name, "spatial", |n| &n.spatial)
		.cloned()
}
pub fn find_spatial_parent(calling_client: &Arc<Client>, node_path: &str) -> Result<Arc<Spatial>> {
	find_spatial(calling_client, "Spatial parent", node_path)
}
pub fn get_spatial(node: &Arc<Node>, node_name: &str) -> Result<Arc<Spatial>> {
	node.get_aspect(node_name, "spatial", |n| &n.spatial)
		.cloned()
}

pub struct SpatialInterface;
impl SpatialInterfaceAspect for SpatialInterface {
	fn create_spatial(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		name: String,
		parent: Arc<Node>,
		transform: Transform,
		zoneable: bool,
	) -> Result<()> {
		let parent = get_spatial(&parent, "Spatial parent")?;
		let transform = parse_transform(transform, true, true, true);
		let node = Node::create_parent_name(&calling_client, "/spatial/spatial", &name, true)
			.add_to_scenegraph()?;
		Spatial::add_to(&node, Some(parent), transform, zoneable)?;
		Ok(())
	}
	fn create_zone(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		name: String,
		parent: Arc<Node>,
		transform: Transform,
		field: Arc<Node>,
	) -> Result<()> {
		let parent = get_spatial(&parent, "Spatial parent")?;
		let transform = parse_transform(transform, true, true, false);
		let field = get_field(&field)?;

		let node = Node::create_parent_name(&calling_client, "/spatial/zone", &name, true)
			.add_to_scenegraph()?;
		let space = Spatial::add_to(&node, Some(parent), transform, false)?;
		Zone::add_to(&node, space, &field);
		Ok(())
	}
}

create_interface!(SpatialInterface, SpatialInterfaceAspect, "/spatial");

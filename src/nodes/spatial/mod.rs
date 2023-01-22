pub mod zone;

use self::zone::{create_zone_flex, Zone};
use super::Node;
use crate::core::client::Client;
use crate::core::registry::Registry;
use color_eyre::eyre::{ensure, eyre, Result};
use glam::{vec3a, Mat4, Quat};
use mint::Vector3;
use nanoid::nanoid;
use parking_lot::Mutex;
use serde::Deserialize;
use stardust_xr::schemas::flex::{deserialize, serialize};
use stardust_xr::values::Transform;
use std::fmt::Debug;
use std::ptr;
use std::sync::{Arc, Weak};
use tracing::instrument;

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
		let spatial = Spatial::new(Arc::downgrade(node), parent, transform);
		node.add_local_method("get_transform", Spatial::get_transform_flex);
		node.add_local_signal("set_transform", Spatial::set_transform_flex);
		node.add_local_signal("set_spatial_parent", Spatial::set_spatial_parent_flex);
		node.add_local_signal(
			"set_spatial_parent_in_place",
			Spatial::set_spatial_parent_in_place_flex,
		);
		node.add_local_signal("set_zoneable", Spatial::set_zoneable);
		if zoneable {
			ZONEABLE_REGISTRY.add_raw(&spatial);
		}
		let _ = node.spatial.set(spatial.clone());
		Ok(spatial)
	}

	#[instrument(level = "debug", skip_all)]
	pub fn space_to_space_matrix(from: Option<&Spatial>, to: Option<&Spatial>) -> Mat4 {
		let space_to_world_matrix = from.map_or(Mat4::IDENTITY, |from| from.global_transform());
		let world_to_space_matrix = to.map_or(Mat4::IDENTITY, |to| to.global_transform().inverse());
		world_to_space_matrix * space_to_world_matrix
	}

	#[instrument(level = "debug", skip_all)]
	pub fn local_transform(&self) -> Mat4 {
		*self.transform.lock()
	}
	pub fn global_transform(&self) -> Mat4 {
		match self.get_parent() {
			Some(value) => value.global_transform() * *self.transform.lock(),
			None => *self.transform.lock(),
		}
	}
	#[instrument]
	pub fn set_local_transform(&self, transform: Mat4) {
		*self.transform.lock() = transform;
	}
	#[instrument(level = "debug", skip(self, reference_space))]
	pub fn set_local_transform_components(
		&self,
		reference_space: Option<&Spatial>,
		transform: Transform,
	) {
		if reference_space == Some(self) {
			self.set_local_transform(
				self.local_transform()
					* Mat4::from_scale_rotation_translation(
						transform.scale.map(|s| s.into()).unwrap_or_default(),
						transform.rotation.map(|r| r.into()).unwrap_or_default(),
						transform.position.map(|t| t.into()).unwrap_or_default(),
					),
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

		if let Some(pos) = transform.position {
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

	#[instrument(level = "debug", skip_all)]
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

	#[instrument(level = "debug", skip_all)]
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

	#[instrument(level = "debug", skip_all)]
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

	pub fn get_transform_flex(
		node: &Node,
		calling_client: Arc<Client>,
		data: &[u8],
	) -> Result<Vec<u8>> {
		let this_spatial = node
			.spatial
			.get()
			.ok_or_else(|| eyre!("Node doesn't have a spatial?"))?;
		let relative_spatial = find_reference_space(&calling_client, deserialize(data)?)?;

		let (scale, rotation, position) = Spatial::space_to_space_matrix(
			Some(this_spatial.as_ref()),
			Some(relative_spatial.as_ref()),
		)
		.to_scale_rotation_translation();

		serialize((
			mint::Vector3::from(position),
			mint::Quaternion::from(rotation),
			mint::Vector3::from(scale),
		))
		.map_err(|e| e.into())
	}
	pub fn set_transform_flex(node: &Node, calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		#[derive(Deserialize)]
		struct TransformArgs<'a> {
			reference_space_path: Option<&'a str>,
			transform: Transform,
		}
		let transform_args: TransformArgs = deserialize(data)?;
		let reference_space_transform = transform_args
			.reference_space_path
			.map(|path| find_reference_space(&calling_client, path))
			.transpose()?;

		node.spatial.get().unwrap().set_local_transform_components(
			reference_space_transform.as_deref(),
			transform_args.transform,
		);
		Ok(())
	}
	pub fn set_spatial_parent_flex(
		node: &Node,
		calling_client: Arc<Client>,
		data: &[u8],
	) -> Result<()> {
		let parent = find_spatial_parent(&calling_client, deserialize(data)?)?;
		node.spatial.get().unwrap().set_spatial_parent(Some(parent))
	}
	pub fn set_spatial_parent_in_place_flex(
		node: &Node,
		calling_client: Arc<Client>,
		data: &[u8],
	) -> Result<()> {
		let parent = find_spatial_parent(&calling_client, deserialize(data)?)?;
		node.spatial
			.get()
			.unwrap()
			.set_spatial_parent_in_place(Some(parent))?;
		Ok(())
	}
	pub fn set_zoneable(node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		let zoneable: bool = deserialize(data)?;
		let spatial = node.spatial.get().unwrap();
		if zoneable {
			ZONEABLE_REGISTRY.add_raw(spatial);
		} else {
			ZONEABLE_REGISTRY.remove(spatial);
			zone::release(spatial);
		}
		Ok(())
	}

	#[instrument]
	pub(self) fn zone_distance(&self) -> f32 {
		self.zone
			.lock()
			.upgrade()
			.and_then(|zone| zone.field.upgrade())
			.map(|field| field.distance(self, vec3a(0.0, 0.0, 0.0)))
			.unwrap_or(f32::MAX)
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

pub fn parse_transform(
	transform: Transform,
	position: bool,
	rotation: bool,
	scale: bool,
) -> Result<Mat4> {
	let position = position
		.then_some(transform.position)
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

	Ok(Mat4::from_scale_rotation_translation(
		scale.into(),
		rotation.into(),
		position.into(),
	))
}

pub fn find_spatial(
	calling_client: &Arc<Client>,
	node_name: &'static str,
	node_path: &str,
) -> color_eyre::eyre::Result<Arc<Spatial>> {
	calling_client
		.get_node(node_name, node_path)?
		.get_aspect(node_name, "spatial", |n| &n.spatial)
		.cloned()
}
pub fn find_spatial_parent(
	calling_client: &Arc<Client>,
	node_path: &str,
) -> color_eyre::eyre::Result<Arc<Spatial>> {
	find_spatial(calling_client, "Spatial parent", node_path)
}
pub fn find_reference_space(
	calling_client: &Arc<Client>,
	node_path: &str,
) -> color_eyre::eyre::Result<Arc<Spatial>> {
	find_spatial(calling_client, "Reference space", node_path)
}

pub fn create_interface(client: &Arc<Client>) {
	let node = Node::create(client, "", "spatial", false);
	node.add_local_signal("create_spatial", create_spatial_flex);
	node.add_local_signal("create_zone", create_zone_flex);
	node.add_to_scenegraph();
}

pub fn create_spatial_flex(_node: &Node, calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
	#[derive(Deserialize)]
	struct CreateSpatialInfo<'a> {
		name: &'a str,
		parent_path: &'a str,
		transform: Transform,
		zoneable: bool,
	}
	let info: CreateSpatialInfo = deserialize(data)?;
	let node = Node::create(&calling_client, "/spatial/spatial", info.name, true);
	let parent = find_spatial_parent(&calling_client, info.parent_path)?;
	let transform = parse_transform(info.transform, true, true, true)?;
	let node = node.add_to_scenegraph();
	Spatial::add_to(&node, Some(parent), transform, info.zoneable)?;
	Ok(())
}

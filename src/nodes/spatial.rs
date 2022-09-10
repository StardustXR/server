use super::core::Node;
use crate::core::client::Client;
use anyhow::{anyhow, ensure, Result};
use glam::{Mat4, Quat, Vec3};
use libstardustxr::flex::flexbuffer_from_vector_arguments;
use libstardustxr::push_to_vec;
use libstardustxr::{flex_to_quat, flex_to_vec3};
use parking_lot::Mutex;
use std::ptr;
use std::sync::{Arc, Weak};

pub struct Spatial {
	pub(super) node: Weak<Node>,
	parent: Mutex<Option<Arc<Spatial>>>,
	pub(super) transform: Mutex<Mat4>,
}

impl Spatial {
	pub fn add_to(
		node: &Arc<Node>,
		parent: Option<Arc<Spatial>>,
		transform: Mat4,
	) -> Result<Arc<Spatial>> {
		ensure!(
			node.spatial.get().is_none(),
			"Internal: Node already has a Spatial aspect!"
		);
		let spatial = Spatial {
			node: Arc::downgrade(node),
			parent: Mutex::new(parent),
			transform: Mutex::new(transform),
		};
		node.add_local_method("getTransform", Spatial::get_transform_flex);
		node.add_local_signal("setTransform", Spatial::set_transform_flex);
		node.add_local_signal("setSpatialParent", Spatial::set_spatial_parent_flex);
		node.add_local_signal(
			"setSpatialParentInPlace",
			Spatial::set_spatial_parent_in_place_flex,
		);
		let spatial_arc = Arc::new(spatial);
		let _ = node.spatial.set(spatial_arc.clone());
		Ok(spatial_arc)
	}

	pub fn space_to_space_matrix(from: Option<&Spatial>, to: Option<&Spatial>) -> Mat4 {
		let space_to_world_matrix = from.map_or(Mat4::IDENTITY, |from| from.global_transform());
		let world_to_space_matrix = to.map_or(Mat4::IDENTITY, |to| to.global_transform().inverse());
		world_to_space_matrix * space_to_world_matrix
	}

	pub fn local_transform(&self) -> Mat4 {
		*self.transform.lock()
	}
	pub fn global_transform(&self) -> Mat4 {
		match self.parent.lock().clone() {
			Some(value) => value.global_transform() * *self.transform.lock(),
			None => *self.transform.lock(),
		}
	}
	pub fn set_local_transform(&self, transform: Mat4) {
		*self.transform.lock() = transform;
	}
	pub fn set_local_transform_components(
		&self,
		reference_space: Option<&Spatial>,
		pos: Option<Vec3>,
		rot: Option<Quat>,
		scl: Option<Vec3>,
	) {
		let reference_to_parent_transform = reference_space
			.map(|reference_space| {
				Spatial::space_to_space_matrix(Some(reference_space), self.parent.lock().as_deref())
			})
			.unwrap_or(Mat4::IDENTITY);
		let mut local_transform_in_reference_space =
			reference_to_parent_transform.inverse() * self.local_transform();
		let (mut reference_space_scl, mut reference_space_rot, mut reference_space_pos) =
			local_transform_in_reference_space.to_scale_rotation_translation();

		if let Some(pos) = pos {
			reference_space_pos = pos
		}
		if let Some(rot) = rot {
			reference_space_rot = rot
		} else if reference_space_rot.is_nan() {
			reference_space_rot = Quat::IDENTITY;
		}
		if let Some(scl) = scl {
			reference_space_scl = scl
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

			let current_ancestor_parent = current_ancestor.parent.lock().clone();
			if let Some(parent) = current_ancestor_parent {
				current_ancestor = parent;
			} else {
				return false;
			}
		}
	}

	pub fn set_spatial_parent(&self, parent: Option<&Arc<Spatial>>) -> Result<()> {
		let is_ancestor = parent
			.map(|parent| self.is_ancestor_of(parent.clone()))
			.unwrap_or(false);
		if is_ancestor {
			return Err(anyhow!("Setting spatial parent would cause a loop"));
		}

		*self.parent.lock() = parent.cloned();

		Ok(())
	}

	pub fn set_spatial_parent_in_place(&self, parent: Option<&Arc<Spatial>>) -> Result<()> {
		let is_ancestor = parent
			.map(|parent| self.is_ancestor_of(parent.clone()))
			.unwrap_or(false);
		if is_ancestor {
			return Err(anyhow!("Setting spatial parent would cause a loop"));
		}

		self.set_local_transform(Spatial::space_to_space_matrix(
			Some(self),
			parent.cloned().as_deref(),
		));
		*self.parent.lock() = parent.cloned();

		Ok(())
	}

	pub fn get_transform_flex(
		node: &Node,
		calling_client: Arc<Client>,
		data: &[u8],
	) -> Result<Vec<u8>> {
		let root = flexbuffers::Reader::get_root(data)?;
		let this_spatial = node
			.spatial
			.get()
			.ok_or_else(|| anyhow!("Node doesn't have a spatial?"))?;
		let relative_spatial = calling_client
			.scenegraph
			.get_node(root.as_str())
			.ok_or_else(|| anyhow!("Space not found"))?
			.spatial
			.get()
			.ok_or_else(|| anyhow!("Reference space node is not a spatial"))?
			.clone();

		let (scale, rotation, position) = Spatial::space_to_space_matrix(
			Some(this_spatial.as_ref()),
			Some(relative_spatial.as_ref()),
		)
		.to_scale_rotation_translation();

		Ok(flexbuffer_from_vector_arguments(|vec| {
			push_to_vec!(
				vec,
				mint::Vector3::from(position),
				mint::Quaternion::from(rotation),
				mint::Vector3::from(scale)
			);
		}))
	}
	pub fn set_transform_flex(node: &Node, calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		let flex_vec = flexbuffers::Reader::get_root(data)?.get_vector()?;
		let reference_space_path = flex_vec.idx(0).as_str();
		let reference_space_transform = if reference_space_path.is_empty() {
			None
		} else {
			Some(
				calling_client
					.scenegraph
					.get_node(reference_space_path)
					.ok_or_else(|| anyhow!("Other spatial node not found"))?
					.spatial
					.get()
					.ok_or_else(|| anyhow!("Node is not a Spatial!"))?
					.clone(),
			)
		};
		node.spatial.get().unwrap().set_local_transform_components(
			reference_space_transform.as_deref(),
			flex_to_vec3!(flex_vec.idx(1)).map(|v| v.into()),
			flex_to_quat!(flex_vec.idx(2)).map(|v| v.into()),
			flex_to_vec3!(flex_vec.idx(3)).map(|v| v.into()),
		);
		Ok(())
	}
	pub fn set_spatial_parent_flex(
		node: &Node,
		calling_client: Arc<Client>,
		data: &[u8],
	) -> Result<()> {
		let parent_path = flexbuffers::Reader::get_root(data)?.get_str()?;
		let parent = calling_client
			.scenegraph
			.get_node(parent_path)
			.ok_or_else(|| anyhow!("Parent node not found"))?
			.spatial
			.get()
			.ok_or_else(|| anyhow!("Parent node is not a Spatial!"))?
			.clone();
		node.spatial
			.get()
			.unwrap()
			.set_spatial_parent(Some(&parent))?;
		Ok(())
	}
	pub fn set_spatial_parent_in_place_flex(
		node: &Node,
		calling_client: Arc<Client>,
		data: &[u8],
	) -> Result<()> {
		let parent_path = flexbuffers::Reader::get_root(data)?.get_str()?;
		let parent = calling_client
			.scenegraph
			.get_node(parent_path)
			.ok_or_else(|| anyhow!("Parent node not found"))?
			.spatial
			.get()
			.ok_or_else(|| anyhow!("Parent node is not a Spatial!"))?
			.clone();
		node.spatial
			.get()
			.unwrap()
			.set_spatial_parent_in_place(Some(&parent))?;
		Ok(())
	}
}

pub fn get_spatial_parent_flex(
	calling_client: &Arc<Client>,
	node_path: &str,
) -> Result<Arc<Spatial>> {
	Ok(calling_client
		.scenegraph
		.get_node(node_path)
		.ok_or_else(|| anyhow!("Spatial parent node not found"))?
		.spatial
		.get()
		.ok_or_else(|| anyhow!("Spatial parent node is not a spatial"))?
		.clone())
}
pub fn get_transform_pose_flex<B: flexbuffers::Buffer>(
	translation: &flexbuffers::Reader<B>,
	rotation: &flexbuffers::Reader<B>,
) -> Result<Mat4> {
	Ok(Mat4::from_rotation_translation(
		flex_to_quat!(rotation)
			.ok_or_else(|| anyhow!("Rotation not found"))?
			.into(),
		flex_to_vec3!(translation)
			.ok_or_else(|| anyhow!("Position not found"))?
			.into(),
	))
}

pub fn create_interface(client: &Arc<Client>) {
	let node = Node::create(client, "", "spatial", false);
	node.add_local_signal("createSpatial", create_spatial_flex);
	node.add_to_scenegraph();
}

pub fn create_spatial_flex(_node: &Node, calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
	let flex_vec = flexbuffers::Reader::get_root(data)?.get_vector()?;
	let node = Node::create(
		&calling_client,
		"/spatial/spatial",
		flex_vec.idx(0).get_str()?,
		true,
	);
	let parent = get_spatial_parent_flex(&calling_client, flex_vec.idx(1).get_str()?)?;
	let transform = Mat4::from_scale_rotation_translation(
		flex_to_vec3!(flex_vec.idx(4))
			.ok_or_else(|| anyhow!("Scale not found"))?
			.into(),
		flex_to_quat!(flex_vec.idx(3))
			.ok_or_else(|| anyhow!("Rotation not found"))?
			.into(),
		flex_to_vec3!(flex_vec.idx(2))
			.ok_or_else(|| anyhow!("Position not found"))?
			.into(),
	);
	let node = node.add_to_scenegraph();
	Spatial::add_to(&node, Some(parent), transform)?;
	Ok(())
}

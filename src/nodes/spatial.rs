use super::core::Node;
use crate::core::client::Client;
use anyhow::{anyhow, ensure, Result};
use glam::{Mat4, Quat, Vec3};
use libstardustxr::flex::flexbuffer_from_vector_arguments;
use libstardustxr::push_to_vec;
use libstardustxr::{flex_to_quat, flex_to_vec3};
use parking_lot::RwLock;
use rccell::RcCell;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

pub struct Spatial {
	// node: WeakCell<Node>,
	parent: RwLock<Option<Arc<Spatial>>>,
	transform: RwLock<Mat4>,
}

impl Spatial {
	pub fn add_to(
		node: &RcCell<Node>,
		parent: Option<Arc<Spatial>>,
		transform: Mat4,
	) -> Result<Arc<Spatial>> {
		ensure!(
			node.borrow_mut().spatial.is_none(),
			"Internal: Node already has a Spatial aspect!"
		);
		let spatial = Spatial {
			// node: node.downgrade(),
			parent: RwLock::new(parent),
			transform: RwLock::new(transform),
		};
		node.borrow_mut()
			.add_local_method("getTransform", Spatial::get_transform_flex);
		node.borrow_mut()
			.add_local_signal("setTransform", Spatial::set_transform_flex);
		let spatial_arc = Arc::new(spatial);
		node.borrow_mut().spatial = Some(spatial_arc.clone());
		Ok(spatial_arc)
	}

	pub fn space_to_space_matrix(from: Option<&Spatial>, to: Option<&Spatial>) -> Mat4 {
		let space_to_world_matrix = from.map_or(Mat4::IDENTITY, |from| from.global_transform());
		let world_to_space_matrix = to.map_or(Mat4::IDENTITY, |to| to.global_transform().inverse());
		world_to_space_matrix * space_to_world_matrix
	}

	pub fn local_transform(&self) -> Mat4 {
		*self.transform.read()
	}
	pub fn global_transform(&self) -> Mat4 {
		match self.parent.read().clone() {
			Some(value) => value.global_transform() * *self.transform.read(),
			None => *self.transform.read(),
		}
	}
	pub fn set_local_transform(&self, transform: Mat4) {
		*self.transform.write() = transform;
	}
	pub fn set_local_transform_components(
		&self,
		reference_space: Option<&Spatial>,
		pos: Option<Vec3>,
		rot: Option<Quat>,
		scl: Option<Vec3>,
	) {
		let reference_to_parent_transform =
			Spatial::space_to_space_matrix(reference_space, self.parent.read().as_deref());
		let mut local_transform_in_reference_space =
			reference_to_parent_transform.inverse() * self.local_transform();
		let (mut reference_space_scl, mut reference_space_rot, mut reference_space_pos) =
			local_transform_in_reference_space.to_scale_rotation_translation();

		if let Some(pos) = pos {
			reference_space_pos = pos
		}
		if let Some(rot) = rot {
			reference_space_rot = rot
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

	pub fn get_transform_flex(
		node: &Node,
		calling_client: Rc<Client>,
		data: &[u8],
	) -> Result<Vec<u8>> {
		let root = flexbuffers::Reader::get_root(data)?;
		let this_spatial = node
			.spatial
			.clone()
			.ok_or_else(|| anyhow!("Node doesn't have a spatial?"))?;
		let relative_spatial = calling_client
			.get_scenegraph()
			.get_node(root.as_str())
			.and_then(|node| node.borrow().spatial.clone())
			.ok_or_else(|| anyhow!("Space not found"))?;

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
	pub fn set_transform_flex(node: &Node, calling_client: Rc<Client>, data: &[u8]) -> Result<()> {
		let root = flexbuffers::Reader::get_root(data)?;
		let flex_vec = root.get_vector()?;
		let spatial = node
			.spatial
			.as_ref()
			.ok_or_else(|| anyhow!("Node somehow is not spatial"))?;
		let reference_space_path = flex_vec.idx(0).as_str();
		let reference_space_transform = if reference_space_path.is_empty() {
			None
		} else {
			Some(
				calling_client
					.get_scenegraph()
					.get_node(reference_space_path)
					.ok_or_else(|| anyhow!("Other spatial node not found"))?
					.borrow()
					.spatial
					.as_ref()
					.ok_or_else(|| anyhow!("Node is not a Spatial!"))?
					.clone(),
			)
		};
		spatial.set_local_transform_components(
			reference_space_transform.as_deref(),
			flex_to_vec3!(flex_vec.idx(1)).map(|v| v.into()),
			flex_to_quat!(flex_vec.idx(2)).map(|v| v.into()),
			flex_to_vec3!(flex_vec.idx(3)).map(|v| v.into()),
		);
		Ok(())
	}
}

pub fn create_interface(client: Rc<Client>) {
	let mut node = Node::create(Rc::downgrade(&client), "", "spatial", false);
	node.add_local_signal("createSpatial", create_spatial_flex);
	client.get_scenegraph().add_node(node);
}

pub fn create_spatial_flex(_node: &Node, calling_client: Rc<Client>, data: &[u8]) -> Result<()> {
	let root = flexbuffers::Reader::get_root(data)?;
	let flex_vec = root.get_vector()?;
	let spatial = Node::create(
		Rc::downgrade(&calling_client),
		"/spatial/spatial",
		flex_vec.idx(0).get_str()?,
		true,
	);
	let parent = calling_client
		.get_scenegraph()
		.get_node(flex_vec.idx(1).as_str())
		.and_then(|node| node.borrow().spatial.clone());
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
	let spatial_rc = calling_client.get_scenegraph().add_node(spatial);
	Spatial::add_to(&spatial_rc, parent, transform)?;
	Ok(())
}

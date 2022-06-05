use super::core::Node;
use crate::core::client::Client;
use anyhow::{anyhow, bail, ensure, Result};
use glam::{Mat4, Quat, Vec3};
use libstardustxr::{flex_to_quat, flex_to_vec3};
use rccell::{RcCell, WeakCell};

pub struct Spatial<'a> {
	node: WeakCell<Node<'a>>,
	parent: WeakCell<Node<'a>>,
	transform: Mat4,
}

impl<'a> Spatial<'a> {
	pub fn add_to(
		node: RcCell<Node<'a>>,
		parent: WeakCell<Node<'a>>,
		transform: Mat4,
	) -> Result<()> {
		if node.borrow_mut().spatial.is_none() {
			bail!("Node already has a Spatial aspect!");
		}
		let spatial = Spatial {
			node: node.downgrade(),
			parent,
			transform,
		};
		let node_captured = node.clone();
		node.borrow_mut().add_local_signal(
			"setTransform",
			Box::new(move |calling_client, data| {
				let root = flexbuffers::Reader::get_root(data)?;
				let flex_vec = root.get_vector()?;
				let client = node_captured
					.borrow()
					.get_client()
					.ok_or(anyhow!("Node somehow has no client!"))?;
				let other_spatial = calling_client
					.borrow()
					.get_scenegraph()
					.nodes
					.get(flex_vec.idx(0).as_str())
					.ok_or(anyhow!("Spatial node not found"))?
					.clone();
				ensure!(
					other_spatial.borrow().spatial.is_some(),
					"Node is not a Spatial!"
				);
				let pos = flex_to_vec3!(flex_vec.idx(1));
				let rot = flex_to_quat!(flex_vec.idx(2));
				let scl = flex_to_vec3!(flex_vec.idx(3));
				node_captured
					.borrow_mut()
					.spatial
					.as_mut()
					.unwrap()
					.set_transform_components(client, other_spatial, pos.into(), rot, scl);
				Ok(())
			}),
		);
		node.borrow_mut().spatial = Some(spatial);
		Ok(())
	}

	pub fn local_transform(&self) -> Mat4 {
		self.transform
	}
	pub fn global_transform(&self) -> Mat4 {
		match self.parent.upgrade() {
			Some(value) => {
				value.borrow().spatial.as_ref().unwrap().global_transform() * self.transform
			}
			None => self.transform,
		}
	}

	pub fn set_transform_components(
		&mut self,
		calling_client: RcCell<Client>,
		relative_space: RcCell<Node>,
		pos: Option<mint::Vector3<f32>>,
		rot: Option<mint::Quaternion<f32>>,
		scl: Option<mint::Vector3<f32>>,
	) {
		todo!()
	}

	// pub fn relative_transform(&self, space: WeakCell<Spatial>) {}
}

pub fn create_interface(client: RcCell<Client>) {
	let mut node = Node::create(client.downgrade(), "", "spatial", false);
	node.add_local_signal(
		"createSpatial",
		Box::new(move |calling_client, data| {
			let root = flexbuffers::Reader::get_root(data)?;
			let flex_vec = root.get_vector()?;
			let node = Node::create(
				calling_client.downgrade(),
				"/spatial",
				flex_vec.idx(0).get_str()?,
				true,
			);
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
			let node_rc = calling_client
				.borrow_mut()
				.get_scenegraph_mut()
				.add_node(node);
			Spatial::add_to(node_rc, WeakCell::new(), transform)?;
			Ok(())
		}),
	);
	client.borrow_mut().get_scenegraph_mut().add_node(node);
}

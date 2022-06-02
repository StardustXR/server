use super::core::Node;
use crate::core::client::Client;
use anyhow::{anyhow, Result};
use glam::{Mat4, Quat, Vec3};
use libstardustxr::{flex_to_quat, flex_to_vec3};
use rccell::{RcCell, WeakCell};

pub struct Spatial<'a> {
	pub node: Node<'a>,
	parent: WeakCell<Spatial<'a>>,
	transform: Mat4,
}

impl<'a> Spatial<'a> {
	pub fn new(
		client: WeakCell<Client<'a>>,
		path: &str,
		transform: Mat4,
	) -> Result<WeakCell<Self>> {
		let spatial = RcCell::new(Spatial {
			node: Node::from_path(client.clone(), path, true).unwrap(),
			parent: WeakCell::new(),
			transform,
		});
		let weak_spatial = spatial.downgrade();
		let captured_spatial = weak_spatial.clone();
		let captured_client = client.clone();
		// node_add_local_signal!(node, "setTransform", Spatial::set_transform_components);
		spatial.borrow_mut().node.add_local_signal(
			"setTransform",
			Box::new(move |calling_client, data| {
				let root = flexbuffers::Reader::get_root(data)?;
				let flex_vec = root.get_vector()?;
				let spatial = captured_spatial
					.upgrade()
					.ok_or(anyhow!("Invalid spatial"))?;
				let client = captured_client.upgrade().ok_or(anyhow!("Invalid client"))?;
				let other_spatial = client
					.borrow()
					.get_scenegraph()
					.spatial_nodes
					.get(flex_vec.idx(0).as_str())
					.ok_or(anyhow!("Spatial not found"))?
					.clone();
				let pos = flex_to_vec3!(flex_vec.idx(1));
				let rot = flex_to_quat!(flex_vec.idx(2));
				let scl = flex_to_vec3!(flex_vec.idx(3));
				spatial.borrow_mut().set_transform_components(
					client,
					other_spatial,
					pos.into(),
					rot,
					scl,
				);
				Ok(())
			}),
		);
		client
			.upgrade()
			.unwrap()
			.borrow_mut()
			.get_scenegraph_mut()
			.spatial_nodes
			.insert(path.to_string(), spatial);
		Ok(weak_spatial)
	}

	pub fn local_transform(&self) -> Mat4 {
		self.transform
	}
	pub fn global_transform(&self) -> Mat4 {
		match self.parent.upgrade() {
			Some(value) => value.borrow().global_transform() * self.transform,
			None => self.transform,
		}
	}

	pub fn set_transform_components(
		&mut self,
		calling_client: RcCell<Client>,
		relative_space: RcCell<Spatial>,
		pos: Option<mint::Vector3<f32>>,
		rot: Option<mint::Quaternion<f32>>,
		scl: Option<mint::Vector3<f32>>,
	) {
		todo!()
	}

	// pub fn relative_transform(&self, space: WeakCell<Spatial>) {}
}

use super::core::Node;
use crate::core::client::Client;
use anyhow::{anyhow, Result};
// use euler::Mat4;
use libstardustxr::{flex_to_quat, flex_to_vec3};
use mint::RowMatrix4;
use rccell::{RcCell, WeakCell};

pub struct Spatial<'a> {
	pub node: Node<'a>,
	parent: WeakCell<Spatial<'a>>,
	transform: RowMatrix4<f32>,
}

impl<'a> Spatial<'a> {
	pub fn new(
		client: WeakCell<Client<'a>>,
		path: &str,
		transform: RowMatrix4<f32>,
	) -> Result<WeakCell<Self>> {
		let spatial = RcCell::new(Spatial {
			node: Node::from_path(client.clone(), path, true).unwrap(),
			parent: WeakCell::new(),
			transform,
		});
		let weak_spatial = spatial.downgrade();
		let captured_spatial = weak_spatial.clone();
		let captured_client = client.clone();
		spatial.borrow_mut().node.add_local_signal(
			"setTransform",
			Box::new(move |data| {
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
					.get(flex_vec.idx(1).as_str())
					.ok_or(anyhow!("Spatial not found"))?;
				let pos = flex_to_vec3!(flex_vec.idx(1));
				let rot = flex_to_quat!(flex_vec.idx(2));
				let scl = flex_to_vec3!(flex_vec.idx(3));
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

	pub fn local_transform(&self) -> RowMatrix4<f32> {
		self.transform
	}
	pub fn global_transform(&self) -> RowMatrix4<f32> {
		todo!()
		// match self.parent.upgrade() {
		// 	Some(value) => Mat4::from(value.borrow().global_transform()) * self.transform,
		// 	None => self.transform,
		// }
	}

	pub fn set_transform_components(&mut self) {
		todo!()
	}

	// pub fn relative_transform(&self, space: WeakCell<Spatial>) {}
}

use super::core::Node;
use crate::core::client::Client;
use anyhow::{anyhow, Result};
use euler::Mat4;
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
		client: Option<&mut Client<'a>>,
		path: &str,
		transform: RowMatrix4<f32>,
	) -> Result<WeakCell<Self>> {
		let mut spatial = Spatial {
			node: Node::from_path(client.as_deref(), path, true).unwrap(),
			parent: WeakCell::new(),
			transform,
		};
		let spatial_cell = RcCell::new(spatial);
		let weak_spatial = spatial_cell.downgrade();
		if let Some(client) = client {
			let weak_spatial = weak_spatial.clone();
			spatial.node.add_local_signal(
				"setTransform",
				Box::new(|data| {
					let root = flexbuffers::Reader::get_root(data).unwrap();
					let flex_vec = root.get_vector().unwrap();
					let spatial = client
						.scenegraph
						.as_ref()
						.unwrap()
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
				.scenegraph
				.as_mut()
				.unwrap()
				.spatial_nodes
				.insert(path.to_string(), spatial_cell);
		}
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

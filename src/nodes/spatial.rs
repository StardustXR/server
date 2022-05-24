use super::core::Node;
use crate::core::client::Client;
use rccell::{RcCell, WeakCell};
use vek::mat::repr_c::row_major::Mat4;

pub struct Spatial<'a> {
	pub node: Node<'a>,
	parent: WeakCell<Spatial<'a>>,
	transform: Mat4<f32>,
}

impl<'a> Spatial<'a> {
	pub fn new(
		client: Option<&mut Client<'a>>,
		path: &str,
		transform: Mat4<f32>,
	) -> WeakCell<Self> {
		let spatial = RcCell::new(Spatial {
			node: Node::from_path(client.as_deref(), path).unwrap(),
			parent: WeakCell::new(),
			transform,
		});
		let weak_spatial = spatial.downgrade();
		client
			.unwrap()
			.scenegraph
			.as_mut()
			.unwrap()
			.spatial_nodes
			.insert(path.to_string(), spatial);
		weak_spatial
	}

	pub fn get_local_transform(&self) -> Mat4<f32> {
		self.transform
	}
	pub fn get_global_transform(&self) -> Mat4<f32> {
		match self.parent.upgrade() {
			Some(value) => value.borrow().get_global_transform() * self.transform,
			None => self.transform,
		}
	}

	// pub fn get_transform(&self, space: NodeRef) {}
}

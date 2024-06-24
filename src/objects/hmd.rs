use crate::{
	core::client::INTERNAL_CLIENT,
	nodes::{
		spatial::{Spatial, EXPORTED_SPATIALS},
		Node,
	},
};
use glam::Mat4;
use std::sync::Arc;
use zbus::{interface, Connection};

pub struct HMD;
impl HMD {
	pub async fn create(connection: &Connection) -> Arc<Spatial> {
		let node = Arc::new(Node::generate(&INTERNAL_CLIENT, false));
		let spatial = Spatial::add_to(&node, None, Mat4::IDENTITY, false);
		EXPORTED_SPATIALS.lock().insert(0, node);
		connection.object_server().at("/hmd", Self).await.unwrap();
		spatial
	}
}
#[interface(name = "org.stardustxr.SpatialRef")]
impl HMD {
	#[zbus(property)]
	pub fn uid(&self) -> u64 {
		0
	}
}

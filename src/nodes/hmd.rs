use super::{
	alias::Alias,
	spatial::{Spatial, SPATIAL_ASPECT_ALIAS_INFO},
	Node,
};
use crate::core::client::{Client, INTERNAL_CLIENT};
use color_eyre::eyre::Result;
use glam::{vec3, Mat4};
use std::sync::Arc;
use stereokit_rust::system::Input;

lazy_static::lazy_static! {
	static ref HMD: Arc<Node> = create();
}

fn create() -> Arc<Node> {
	let node = Arc::new(Node::generate(&INTERNAL_CLIENT, false));
	Spatial::add_to(&node, None, Mat4::IDENTITY, false);
	node
}

pub fn frame() {
	let spatial = HMD.get_aspect::<Spatial>().unwrap();
	let hmd_pose = Input::get_head();
	spatial.set_local_transform(Mat4::from_scale_rotation_translation(
		vec3(1.0, 1.0, 1.0),
		hmd_pose.orientation.into(),
		hmd_pose.position.into(),
	));
}

pub fn make_alias(client: &Arc<Client>) -> Result<Arc<Node>> {
	Alias::create(&HMD, client, SPATIAL_ASPECT_ALIAS_INFO.clone(), None)
}

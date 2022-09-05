use super::{
	core::{Alias, Node},
	spatial::Spatial,
};
use crate::core::client::{Client, INTERNAL_CLIENT};
use glam::{vec3, Mat4};
use std::sync::Arc;
use stereokit::StereoKit;

lazy_static::lazy_static! {
	static ref HMD: Arc<Node> = create();
}

fn create() -> Arc<Node> {
	let node = Arc::new(Node::create(&INTERNAL_CLIENT, "", "hmd", false));
	Spatial::add_to(&node, None, Mat4::IDENTITY).unwrap();

	node
}

pub fn frame(sk: &StereoKit) {
	let spatial = HMD.spatial.get().unwrap();
	let hmd_pose = sk.input_head();
	*spatial.transform.lock() = Mat4::from_scale_rotation_translation(
		vec3(1.0, 1.0, 1.0),
		hmd_pose.orientation.into(),
		hmd_pose.position.into(),
	);
}

pub fn make_alias(client: &Arc<Client>) -> Arc<Node> {
	let node = Node::create(client, "", "hmd", false).add_to_scenegraph();
	Alias::add_to(&node, &HMD, vec!["getTransform"], vec![], vec![], vec![]);
	node
}

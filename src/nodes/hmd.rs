use super::{alias::Alias, spatial::Spatial, Node};
use crate::{
	core::client::{Client, INTERNAL_CLIENT},
	nodes::alias::AliasInfo,
};
use glam::{vec3, Mat4};
use std::sync::Arc;
use stereokit::StereoKit;

lazy_static::lazy_static! {
	static ref HMD: Arc<Node> = create();
}

fn create() -> Arc<Node> {
	let node = Arc::new(Node::create(&INTERNAL_CLIENT, "", "hmd", false));
	Spatial::add_to(&node, None, Mat4::IDENTITY, false).unwrap();

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

pub fn make_alias(client: &Arc<Client>) -> Option<Arc<Node>> {
	Alias::create(
		client,
		"",
		"hmd",
		&HMD,
		AliasInfo {
			local_signals: vec!["get_transform"],
			..Default::default()
		},
	)
}

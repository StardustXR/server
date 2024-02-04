use super::{alias::Alias, spatial::Spatial, Node};
use crate::{
	core::client::{Client, INTERNAL_CLIENT},
	nodes::alias::AliasInfo,
};
use color_eyre::eyre::Result;
use glam::{vec3, Mat4};
use std::sync::Arc;
use stereokit::StereoKitMultiThread;

lazy_static::lazy_static! {
	static ref HMD: Arc<Node> = create();
}

fn create() -> Arc<Node> {
	let node = Arc::new(Node::create_parent_name(&INTERNAL_CLIENT, "", "hmd", false));
	Spatial::add_to(&node, None, Mat4::IDENTITY, false).expect("Unable to make spatial for HMD");

	node
}

pub fn frame(sk: &impl StereoKitMultiThread) {
	let spatial = HMD
		.spatial
		.get()
		.expect("Unable to get spatial to update HMD");
	let hmd_pose = sk.input_head();
	*spatial.transform.lock() = Mat4::from_scale_rotation_translation(
		vec3(1.0, 1.0, 1.0),
		hmd_pose.orientation.into(),
		hmd_pose.position.into(),
	);
}

pub fn make_alias(client: &Arc<Client>) -> Result<Arc<Node>> {
	Alias::create(
		client,
		"",
		"hmd",
		&HMD,
		AliasInfo {
			server_signals: vec!["get_bounds", "get_transform"],
			..Default::default()
		},
	)
}

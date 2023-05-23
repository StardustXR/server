use crate::{
	core::client::INTERNAL_CLIENT,
	nodes::{
		input::{pointer::Pointer, InputMethod, InputType},
		spatial::Spatial,
		Node,
	},
};
use color_eyre::eyre::Result;
use glam::Mat4;
use nanoid::nanoid;
use serde::Serialize;
use stardust_xr::schemas::{flat::Datamap, flex::flexbuffers};
use std::sync::Arc;
use stereokit::StereoKitMultiThread;
use tracing::instrument;

#[derive(Debug, Clone, Serialize)]
pub struct KeyboardEvent {
	pub keyboard: String,
	pub keymap: Option<String>,
	pub keys_up: Option<Vec<u32>>,
	pub keys_down: Option<Vec<u32>>,
}

pub struct EyePointer {
	spatial: Arc<Spatial>,
	pointer: Arc<InputMethod>,
}
impl EyePointer {
	pub fn new() -> Result<Self> {
		let node = Node::create(&INTERNAL_CLIENT, "", &nanoid!(), false).add_to_scenegraph()?;
		let spatial = Spatial::add_to(&node, None, Mat4::IDENTITY, false).unwrap();
		let pointer =
			InputMethod::add_to(&node, InputType::Pointer(Pointer::default()), None).unwrap();

		Ok(EyePointer { spatial, pointer })
	}
	#[instrument(level = "debug", name = "Update Flatscreen Pointer Ray", skip_all)]
	pub fn update(&self, sk: &impl StereoKitMultiThread) {
		let ray = sk.input_eyes();
		self.spatial
			.set_local_transform(Mat4::from_rotation_translation(
				ray.orientation,
				ray.position,
			));
		{
			// Set pointer input datamap
			let mut fbb = flexbuffers::Builder::default();
			let mut map = fbb.start_map();
			map.push("eye", 2);
			map.end_map();
			*self.pointer.datamap.lock() = Datamap::new(fbb.take_buffer()).ok();
		}
	}
}

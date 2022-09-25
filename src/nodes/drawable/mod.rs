pub mod model;

use super::Node;
use crate::core::{client::Client, registry::Registry};
use std::sync::Arc;
use stereokit::{lifecycle::DrawContext, StereoKit};

pub trait Drawable: Send + Sync {
	fn draw(&self, sk: &StereoKit, draw_ctx: &DrawContext);
}

pub static DRAWABLE_REGISTRY: Registry<dyn Drawable> = Registry::new();

pub fn create_interface(client: &Arc<Client>) {
	let node = Node::create(client, "", "drawable", false);
	node.add_local_signal("createModelFromFile", model::create_from_file);
	node.add_local_signal("createModelFromResource", model::create_from_resource);
	node.add_to_scenegraph();
}

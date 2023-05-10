pub mod lines;
pub mod model;
pub mod text;

use self::{
	lines::Lines,
	model::{Model, ModelPart},
	text::Text,
};

use super::Node;
use crate::core::client::Client;
use color_eyre::eyre::Result;
use parking_lot::Mutex;
use serde::Deserialize;
use stardust_xr::schemas::flex::deserialize;
use std::{path::PathBuf, sync::Arc};
use stereokit::StereoKitDraw;
use tracing::instrument;

pub fn create_interface(client: &Arc<Client>) -> Result<()> {
	let node = Node::create(client, "", "drawable", false);
	node.add_local_signal("create_lines", lines::create_flex);
	node.add_local_signal("create_model", model::create_flex);
	node.add_local_signal("create_text", text::create_flex);
	node.add_local_signal("set_sky_file", set_sky_file_flex);
	node.add_to_scenegraph().map(|_| ())
}

pub enum Drawable {
	Lines(Arc<Lines>),
	Model(Arc<Model>),
	ModelPart(Arc<ModelPart>),
	Text(Arc<Text>),
}

#[instrument(level = "debug", skip(sk))]
pub fn draw(sk: &impl StereoKitDraw) {
	lines::draw_all(sk);
	model::draw_all(sk);
	text::draw_all(sk);

	if let Some(skytex) = QUEUED_SKYTEX.lock().take() {
		if let Ok((_skylight, skytex)) = sk.tex_create_cubemap_file(&skytex, true, i32::MAX) {
			sk.render_set_skytex(&skytex);
		}
	}
	if let Some(skylight) = QUEUED_SKYLIGHT.lock().take() {
		if let Ok((skylight, _)) = sk.tex_create_cubemap_file(&skylight, true, i32::MAX) {
			sk.render_set_skylight(skylight);
		}
	}
}

static QUEUED_SKYLIGHT: Mutex<Option<PathBuf>> = Mutex::new(None);
static QUEUED_SKYTEX: Mutex<Option<PathBuf>> = Mutex::new(None);

fn set_sky_file_flex(_node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
	#[derive(Deserialize)]
	struct SkyFileInfo {
		path: PathBuf,
		skytex: Option<bool>,
		skylight: Option<bool>,
	}
	let info: SkyFileInfo = deserialize(data)?;
	info.path.metadata()?;
	if info.skytex.unwrap_or_default() {
		QUEUED_SKYTEX.lock().replace(info.path.clone());
	}
	if info.skylight.unwrap_or_default() {
		QUEUED_SKYLIGHT.lock().replace(info.path);
	}

	Ok(())
}

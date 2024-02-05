pub mod lines;
pub mod model;
pub mod shaders;
pub mod text;

use self::{lines::Lines, model::Model, text::Text};
use super::{
	spatial::{Spatial, Transform},
	Node,
};
use crate::{
	core::{client::Client, resource::get_resource_file},
	create_interface,
};
use color_eyre::eyre::{self, Result};
use parking_lot::Mutex;
use stardust_xr::values::ResourceID;
use std::{ffi::OsStr, path::PathBuf, sync::Arc};
use stereokit::StereoKitDraw;

// #[instrument(level = "debug", skip(sk))]
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

stardust_xr_server_codegen::codegen_drawable_protocol!();
create_interface!(DrawableInterface, DrawableInterfaceAspect, "/drawable");

pub struct DrawableInterface;
impl DrawableInterfaceAspect for DrawableInterface {
	#[doc = "Set the sky lignt/texture to a given HDRI file."]
	fn set_sky(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		tex: Option<ResourceID>,
		light: Option<ResourceID>,
	) -> Result<()> {
		if let Some(tex) = tex {
			let resource_path = get_resource_file(&tex, &calling_client, &[OsStr::new("hdr")])
				.ok_or(eyre::eyre!("Could not find resource"))?;
			QUEUED_SKYTEX.lock().replace(resource_path);
		}
		if let Some(light) = light {
			let resource_path = get_resource_file(&light, &calling_client, &[OsStr::new("hdr")])
				.ok_or(eyre::eyre!("Could not find resource"))?;
			QUEUED_SKYLIGHT.lock().replace(resource_path);
		}
		Ok(())
	}

	#[doc = "Create a lines node"]
	fn create_lines(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		name: String,
		parent: Arc<Node>,
		transform: Transform,
		lines: Vec<Line>,
	) -> Result<()> {
		let node =
			Node::create_parent_name(&calling_client, Self::CREATE_LINES_PARENT_PATH, &name, true);
		let parent = parent.get_aspect::<Spatial>()?;
		let transform = transform.to_mat4(true, true, true);

		let node = node.add_to_scenegraph()?;
		Spatial::add_to(&node, Some(parent.clone()), transform, false);
		Lines::add_to(&node, lines)?;
		Ok(())
	}

	#[doc = "Load a GLTF model into a Model node"]
	fn load_model(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		name: String,
		parent: Arc<Node>,
		transform: Transform,
		model: ResourceID,
	) -> Result<()> {
		let node =
			Node::create_parent_name(&calling_client, Self::LOAD_MODEL_PARENT_PATH, &name, true);
		let parent = parent.get_aspect::<Spatial>()?;
		let transform = transform.to_mat4(true, true, true);
		let node = node.add_to_scenegraph()?;
		Spatial::add_to(&node, Some(parent.clone()), transform, false);
		Model::add_to(&node, model)?;
		Ok(())
	}

	#[doc = "Create a text node"]
	fn create_text(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		name: String,
		parent: Arc<Node>,
		transform: Transform,
		text: String,
		style: TextStyle,
	) -> Result<()> {
		let node =
			Node::create_parent_name(&calling_client, Self::CREATE_TEXT_PARENT_PATH, &name, true);
		let parent = parent.get_aspect::<Spatial>()?;
		let transform = transform.to_mat4(true, true, true);

		let node = node.add_to_scenegraph()?;
		Spatial::add_to(&node, Some(parent.clone()), transform, false);
		Text::add_to(&node, text, style)?;
		Ok(())
	}
}

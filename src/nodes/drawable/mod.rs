pub mod lines;
pub mod model;
#[cfg(feature = "wayland")]
pub mod shader_manipulation;
pub mod shaders;
pub mod text;

use self::{lines::Lines, model::Model, text::Text};
use super::{
	spatial::{Spatial, Transform},
	Node,
};
use crate::nodes::spatial::SPATIAL_ASPECT_ALIAS_INFO;
use crate::{
	core::{client::Client, resource::get_resource_file},
	create_interface,
};
use color_eyre::eyre::{self, Result};
use parking_lot::Mutex;
use stardust_xr::values::ResourceID;
use std::{ffi::OsStr, path::PathBuf, sync::Arc};
use stereokit_rust::{sk::MainThreadToken, system::Renderer, tex::SHCubemap};

// #[instrument(level = "debug", skip(sk))]
pub fn draw(token: &MainThreadToken) {
	lines::draw_all(token);
	model::draw_all(token);
	text::draw_all(token);

	if let Some(skytex) = QUEUED_SKYTEX.lock().take() {
		if let Ok(skytex) = SHCubemap::from_cubemap_equirectangular(skytex, true, 100) {
			Renderer::skytex(skytex.tex);
		}
	}
	if let Some(skylight) = QUEUED_SKYLIGHT.lock().take() {
		if let Ok(skylight) = SHCubemap::from_cubemap_equirectangular(skylight, true, 100) {
			Renderer::skylight(skylight.sh);
		}
	}
}

static QUEUED_SKYLIGHT: Mutex<Option<PathBuf>> = Mutex::new(None);
static QUEUED_SKYTEX: Mutex<Option<PathBuf>> = Mutex::new(None);

stardust_xr_server_codegen::codegen_drawable_protocol!();
create_interface!(DrawableInterface);

pub struct DrawableInterface;
impl InterfaceAspect for DrawableInterface {
	fn set_sky_tex(_node: Arc<Node>, calling_client: Arc<Client>, tex: ResourceID) -> Result<()> {
		let resource_path = get_resource_file(&tex, &calling_client, &[OsStr::new("hdr")])
			.ok_or(eyre::eyre!("Could not find resource"))?;
		QUEUED_SKYTEX.lock().replace(resource_path);
		Ok(())
	}

	fn set_sky_light(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		light: ResourceID,
	) -> Result<()> {
		let resource_path = get_resource_file(&light, &calling_client, &[OsStr::new("hdr")])
			.ok_or(eyre::eyre!("Could not find resource"))?;
		QUEUED_SKYLIGHT.lock().replace(resource_path);
		Ok(())
	}

	fn create_lines(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		id: u64,
		parent: Arc<Node>,
		transform: Transform,
		lines: Vec<Line>,
	) -> Result<()> {
		let node = Node::from_id(&calling_client, id, true);
		let parent = parent.get_aspect::<Spatial>()?;
		let transform = transform.to_mat4(true, true, true);

		let node = node.add_to_scenegraph()?;
		Spatial::add_to(&node, Some(parent.clone()), transform, false);
		Lines::add_to(&node, lines)?;
		Ok(())
	}

	fn load_model(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		id: u64,
		parent: Arc<Node>,
		transform: Transform,
		model: ResourceID,
	) -> Result<()> {
		let node = Node::from_id(&calling_client, id, true);
		let parent = parent.get_aspect::<Spatial>()?;
		let transform = transform.to_mat4(true, true, true);
		let node = node.add_to_scenegraph()?;
		Spatial::add_to(&node, Some(parent.clone()), transform, false);
		Model::add_to(&node, model)?;
		Ok(())
	}

	fn create_text(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		id: u64,
		parent: Arc<Node>,
		transform: Transform,
		text: String,
		style: TextStyle,
	) -> Result<()> {
		let node = Node::from_id(&calling_client, id, true);
		let parent = parent.get_aspect::<Spatial>()?;
		let transform = transform.to_mat4(true, true, true);

		let node = node.add_to_scenegraph()?;
		Spatial::add_to(&node, Some(parent.clone()), transform, false);
		Text::add_to(&node, text, style)?;
		Ok(())
	}
}

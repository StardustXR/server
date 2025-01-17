pub mod lines;
pub mod model;
pub mod text;

use self::{lines::Lines, model::Model, text::Text};
use super::{
	spatial::{Spatial, Transform},
	Aspect, AspectIdentifier, Node,
};
use crate::core::{
	client::Client,
	error::{Result, ServerError},
	resource::get_resource_file,
};
use crate::nodes::spatial::SPATIAL_ASPECT_ALIAS_INFO;
use model::ModelPart;
use parking_lot::Mutex;
use stardust_xr::values::ResourceID;
use std::{ffi::OsStr, path::PathBuf, sync::Arc};

static QUEUED_SKYLIGHT: Mutex<Option<PathBuf>> = Mutex::new(None);
static QUEUED_SKYTEX: Mutex<Option<PathBuf>> = Mutex::new(None);

stardust_xr_server_codegen::codegen_drawable_protocol!();

impl AspectIdentifier for Lines {
	impl_aspect_for_lines_aspect_id! {}
}
impl Aspect for Lines {
	impl_aspect_for_lines_aspect! {}
}
impl AspectIdentifier for Model {
	impl_aspect_for_model_aspect_id! {}
}
impl Aspect for Model {
	impl_aspect_for_model_aspect! {}
}
impl AspectIdentifier for ModelPart {
	impl_aspect_for_model_part_aspect_id! {}
}
impl Aspect for ModelPart {
	impl_aspect_for_model_part_aspect! {}
}
impl AspectIdentifier for Text {
	impl_aspect_for_text_aspect_id! {}
}
impl Aspect for Text {
	impl_aspect_for_text_aspect! {}
}

impl InterfaceAspect for Interface {
	fn set_sky_tex(_node: Arc<Node>, calling_client: Arc<Client>, tex: ResourceID) -> Result<()> {
		let resource_path = get_resource_file(&tex, &calling_client, &[OsStr::new("hdr")])
			.ok_or(ServerError::NoResource)?;
		QUEUED_SKYTEX.lock().replace(resource_path);
		Ok(())
	}

	fn set_sky_light(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		light: ResourceID,
	) -> Result<()> {
		let resource_path = get_resource_file(&light, &calling_client, &[OsStr::new("hdr")])
			.ok_or(ServerError::NoResource)?;
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

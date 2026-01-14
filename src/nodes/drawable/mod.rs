pub mod dmatex;
pub mod lines;
pub mod model;
pub mod sky;
pub mod text;

use self::{lines::Lines, model::Model, text::Text};
use super::{
	Aspect, AspectIdentifier, Node,
	spatial::{Spatial, Transform},
};
use crate::{
	core::vulkano_data::VULKANO_CONTEXT,
	nodes::{drawable::dmatex::ALL_DRM_FOURCCS, spatial::SPATIAL_ASPECT_ALIAS_INFO},
};
use crate::{
	core::{Id, client::Client, error::Result, resource::get_resource_file},
	nodes::drawable::dmatex::ImportedDmatex,
};
use color_eyre::eyre::eyre;
use model::ModelPart;
use parking_lot::Mutex;
use stardust_xr_server_foundation::bail;
use stardust_xr_wire::{fd::ProtocolFd, values::ResourceID};
use std::{
	ffi::OsStr,
	path::PathBuf,
	sync::{Arc, OnceLock},
};
use vulkano::format::Format;

static QUEUED_SKYLIGHT: Mutex<Option<Option<PathBuf>>> = Mutex::new(None);
static QUEUED_SKYTEX: Mutex<Option<Option<PathBuf>>> = Mutex::new(None);

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
	fn set_sky_tex(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		tex: Option<ResourceID>,
	) -> Result<()> {
		let resource_path = tex
			.map(|tex| {
				get_resource_file(
					&tex,
					calling_client.base_resource_prefixes.lock().iter(),
					&[OsStr::new("hdr"), OsStr::new("png"), OsStr::new("jpg")],
				)
				.ok_or(eyre!("Could not find resource"))
			})
			.transpose()?;
		QUEUED_SKYTEX.lock().replace(resource_path);
		Ok(())
	}

	fn set_sky_light(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		light: Option<ResourceID>,
	) -> Result<()> {
		let resource_path = light
			.map(|light| {
				get_resource_file(
					&light,
					calling_client.base_resource_prefixes.lock().iter(),
					&[OsStr::new("hdr"), OsStr::new("png"), OsStr::new("jpg")],
				)
				.ok_or(eyre!("Could not find resource"))
			})
			.transpose()?;
		QUEUED_SKYLIGHT.lock().replace(resource_path);
		Ok(())
	}

	fn create_lines(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		id: Id,
		parent: Arc<Node>,
		transform: Transform,
		lines: Vec<Line>,
	) -> Result<()> {
		let node = Node::from_id(&calling_client, id, true);
		let parent = parent.get_aspect::<Spatial>()?;
		let transform = transform.to_mat4(true, true, true);

		let node = node.add_to_scenegraph()?;
		Spatial::add_to(&node, Some(parent.clone()), transform);
		Lines::add_to(&node, lines)?;
		Ok(())
	}

	fn load_model(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		id: Id,
		parent: Arc<Node>,
		transform: Transform,
		model: ResourceID,
	) -> Result<()> {
		let node = Node::from_id(&calling_client, id, true);
		let parent = parent.get_aspect::<Spatial>()?;
		let transform = transform.to_mat4(true, true, true);
		let node = node.add_to_scenegraph()?;
		Spatial::add_to(&node, Some(parent.clone()), transform);
		Model::add_to(&node, model)?;
		Ok(())
	}

	fn create_text(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		id: Id,
		parent: Arc<Node>,
		transform: Transform,
		text: String,
		style: TextStyle,
	) -> Result<()> {
		let node = Node::from_id(&calling_client, id, true);
		let parent = parent.get_aspect::<Spatial>()?;
		let transform = transform.to_mat4(true, true, true);

		let node = node.add_to_scenegraph()?;
		Spatial::add_to(&node, Some(parent.clone()), transform);
		Text::add_to(&node, text, style)?;
		Ok(())
	}

	fn import_dmatex(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		dmatex_id: Id,
		size: DmatexSize,
		format: u32,
		modifier: Id,
		srgb: bool,
		array_layers: Option<u32>,
		planes: Vec<DmatexPlane>,
		timeline_syncobj_fd: ProtocolFd,
	) -> Result<()> {
		let dmatex = ImportedDmatex::new(
			size,
			format,
			modifier.0,
			srgb,
			array_layers,
			planes,
			timeline_syncobj_fd.0,
		)?;
		calling_client.dmatexes.insert(dmatex_id, dmatex);
		Ok(())
	}

	async fn export_dmatex_uid(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		dmatex_id: Id,
	) -> Result<Id> {
		let Some(tex) = calling_client.dmatexes.get(&dmatex_id) else {
			bail!("invalid dmatex id");
		};
		Ok(tex.export_uid().into())
	}

	fn import_dmatex_uid(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		dmatex_id: Id,
		dmatex_uid: Id,
	) -> Result<()> {
		let Some(tex) = ImportedDmatex::import_uid(dmatex_uid.0) else {
			bail!("invalid dmatex id");
		};
		calling_client.dmatexes.insert(dmatex_id, tex);
		Ok(())
	}

	fn unregister_dmatex(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		dmatex_id: Id,
	) -> Result<()> {
		calling_client.dmatexes.remove(&dmatex_id);
		Ok(())
	}

	async fn get_primary_render_device_id(
		_node: Arc<Node>,
		_calling_client: Arc<Client>,
	) -> Result<Id> {
		let vk = VULKANO_CONTEXT.wait();
		let Some(id) = vk.get_drm_render_node_id() else {
			bail!("unable to get render_node id");
		};
		Ok(id.into())
	}

	async fn enumerate_dmatex_formats(
		_node: Arc<Node>,
		_calling_client: Arc<Client>,
		render_node_id: Id,
	) -> Result<Vec<DmatexFormatInfo>> {
		let vk = VULKANO_CONTEXT.wait();
		if Some(render_node_id.0) != vk.get_drm_render_node_id() {
			bail!(
				"enumerating formats for devices other than the render_node used by the server is not implemented yet"
			);
		}
		DMATEX_FORMAT_CACHE.get_or_init(|| {
			// This is slow, but only runs once!
			ALL_DRM_FOURCCS
				.iter()
				.filter_map(|fourcc| {
					let f = Format::try_from(bevy_dmabuf::format_mapping::drm_fourcc_to_vk_format(
						*fourcc,
					)?)
					.ok()?;
					if bevy_dmabuf::format_mapping::vk_format_to_drm_fourcc(f.into())? != *fourcc {
						return None;
					}
					bevy_dmabuf::wgpu_init::vulkan_to_wgpu(f.into())?;
					let props = vk.phys_dev.format_properties(f).ok()?;
					let can_do_srgb =
						bevy_dmabuf::format_mapping::vk_format_to_srgb(f.into()).is_some();
					Some(
						if can_do_srgb {
							props.drm_format_modifier_properties.clone()
						} else {
							Vec::new()
						}
						.into_iter()
						.map(move |v| DmatexFormatInfo {
							format: *fourcc as u32,
							drm_modifier: v.drm_format_modifier.into(),
							is_srgb: true,
							planes: v.drm_format_modifier_plane_count,
						})
						.chain(
							props
								.drm_format_modifier_properties
								.into_iter()
								.map(move |v| DmatexFormatInfo {
									format: *fourcc as u32,
									drm_modifier: v.drm_format_modifier.into(),
									is_srgb: false,
									planes: v.drm_format_modifier_plane_count,
								}),
						),
					)
				})
				.flatten()
				.collect()
		});
		// not a fan of having to call clone here, not sure if theres a better solution
		Ok(DMATEX_FORMAT_CACHE.get().unwrap().clone())
	}
}
static DMATEX_FORMAT_CACHE: OnceLock<Vec<DmatexFormatInfo>> = OnceLock::new();

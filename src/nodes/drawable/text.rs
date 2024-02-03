use crate::{
	core::{client::Client, destroy_queue, registry::Registry, resource::ResourceID},
	nodes::{
		drawable::Drawable,
		spatial::{find_spatial_parent, parse_transform, Spatial, Transform},
		Message, Node,
	},
};
use color_eyre::eyre::{bail, ensure, eyre, Result};
use glam::{vec3, Mat4, Vec2};
use mint::Vector2;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use portable_atomic::{AtomicBool, Ordering};
use prisma::{Flatten, Rgba};
use serde::Deserialize;
use stardust_xr::schemas::flex::deserialize;
use std::{ffi::OsStr, path::PathBuf, sync::Arc};
use stereokit::{named_colors::WHITE, Color128, StereoKitDraw, TextAlign, TextFit, TextStyle};

static TEXT_REGISTRY: Registry<Text> = Registry::new();

struct TextData {
	text: String,
	character_height: f32,
	text_align: TextAlign,
	bounds: Option<Vec2>,
	fit: TextFit,
	bounds_align: TextAlign,
	color: Rgba<f32>,
}

pub struct Text {
	enabled: Arc<AtomicBool>,
	space: Arc<Spatial>,
	font_path: Option<PathBuf>,
	style: OnceCell<TextStyle>,

	data: Mutex<TextData>,
}
impl Text {
	#[allow(clippy::too_many_arguments)]
	pub fn add_to(
		node: &Arc<Node>,
		font_resource_id: Option<ResourceID>,
		text: String,
		character_height: f32,
		text_align: TextAlign,
		bounds: Option<Vector2<f32>>,
		fit: TextFit,
		bounds_align: TextAlign,
		color: Rgba<f32>,
	) -> Result<Arc<Text>> {
		ensure!(
			node.spatial.get().is_some(),
			"Internal: Node does not have a spatial attached!"
		);
		ensure!(
			node.drawable.get().is_none(),
			"Internal: Node already has a drawable attached!"
		);

		let client = node.get_client().ok_or_else(|| eyre!("Client not found"))?;
		let text = TEXT_REGISTRY.add(Text {
			enabled: node.enabled.clone(),
			space: node.spatial.get().unwrap().clone(),
			font_path: font_resource_id.and_then(|res| {
				res.get_file(
					&client.base_resource_prefixes.lock().clone(),
					&[OsStr::new("ttf"), OsStr::new("otf")],
				)
			}),
			style: OnceCell::new(),

			data: Mutex::new(TextData {
				text,
				character_height,
				text_align,
				bounds: bounds.map(|b| b.into()),
				fit,
				bounds_align,
				color,
			}),
		});
		node.add_local_signal("set_character_height", Text::set_character_height_flex);
		node.add_local_signal("set_text", Text::set_text_flex);
		let _ = node.drawable.set(Drawable::Text(text.clone()));

		Ok(text)
	}

	fn draw(&self, sk: &impl StereoKitDraw) {
		let style = self
			.style
			.get_or_try_init(|| -> Result<TextStyle, color_eyre::eyre::Error> {
				let font = self
					.font_path
					.as_deref()
					.and_then(|path| sk.font_create(path).ok())
					.unwrap_or_else(|| sk.font_find("default/font").unwrap());
				Ok(unsafe { sk.text_make_style(font, 1.0, WHITE) })
			});

		if let Ok(style) = style {
			let data = self.data.lock();
			let transform = self.space.global_transform()
				* Mat4::from_scale(vec3(
					data.character_height,
					data.character_height,
					data.character_height,
				));
			if let Some(bounds) = data.bounds {
				sk.text_add_in(
					&data.text,
					transform,
					bounds / data.character_height,
					data.fit,
					*style,
					data.bounds_align,
					data.text_align,
					vec3(0.0, 0.0, 0.0),
					Color128::from([
						data.color.red(),
						data.color.green(),
						data.color.blue(),
						data.color.alpha(),
					]),
				);
			} else {
				sk.text_add_at(
					&data.text,
					transform,
					*style,
					data.bounds_align,
					data.text_align,
					vec3(0.0, 0.0, 0.0),
					Color128::from([
						data.color.red(),
						data.color.green(),
						data.color.blue(),
						data.color.alpha(),
					]),
				);
			}
		}
	}

	pub fn set_character_height_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		message: Message,
	) -> Result<()> {
		let Some(Drawable::Text(text)) = node.drawable.get() else {
			bail!("Not a drawable??")
		};

		text.data.lock().character_height = deserialize(message.as_ref())?;
		Ok(())
	}

	pub fn set_text_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		message: Message,
	) -> Result<()> {
		let Some(Drawable::Text(text)) = node.drawable.get() else {
			bail!("Not a drawable??")
		};

		text.data.lock().text = deserialize(message.as_ref())?;
		Ok(())
	}
}
impl Drop for Text {
	fn drop(&mut self) {
		if let Some(style) = self.style.take() {
			destroy_queue::add(style);
		}
		TEXT_REGISTRY.remove(self);
	}
}

pub fn draw_all(sk: &impl StereoKitDraw) {
	for text in TEXT_REGISTRY.get_valid_contents() {
		if text.enabled.load(Ordering::Relaxed) {
			text.draw(sk);
		}
	}
}

pub fn create_flex(_node: &Node, calling_client: Arc<Client>, message: Message) -> Result<()> {
	#[derive(Deserialize)]
	struct CreateTextInfo<'a> {
		name: &'a str,
		parent_path: &'a str,
		transform: Transform,
		text: String,
		font_resource: Option<ResourceID>,
		character_height: f32,
		text_align: TextAlign,
		bounds: Option<Vector2<f32>>,
		fit: TextFit,
		bounds_align: TextAlign,
		color: [f32; 4],
	}
	let info: CreateTextInfo = deserialize(message.as_ref())?;
	let node = Node::create(&calling_client, "/drawable/text", info.name, true);
	let parent = find_spatial_parent(&calling_client, info.parent_path)?;
	let transform = parse_transform(info.transform, true, true, true);
	let color = Rgba::from_slice(&info.color);

	let node = node.add_to_scenegraph()?;
	Spatial::add_to(&node, Some(parent), transform, false)?;
	Text::add_to(
		&node,
		info.font_resource,
		info.text,
		info.character_height,
		info.text_align,
		info.bounds,
		info.fit,
		info.bounds_align,
		color,
	)?;
	Ok(())
}

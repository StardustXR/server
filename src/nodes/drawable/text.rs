use crate::{
	core::{client::Client, destroy_queue, registry::Registry, resource::ResourceID},
	nodes::{
		spatial::{find_spatial_parent, parse_transform, Spatial},
		Node,
	},
};
use color_eyre::eyre::{ensure, eyre, Result};
use glam::{vec3, Mat4, Vec2};
use mint::Vector2;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use prisma::{Flatten, Rgba};
use send_wrapper::SendWrapper;
use serde::Deserialize;
use stardust_xr::{schemas::flex::deserialize, values::Transform};
use std::{ffi::OsStr, path::PathBuf, sync::Arc};
use stereokit::{
	color_named::WHITE,
	font::Font,
	lifecycle::StereoKitDraw,
	text::{self, TextAlign, TextFit, TextStyle},
	values::Color128,
};

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
	space: Arc<Spatial>,
	font_path: Option<PathBuf>,
	style: OnceCell<SendWrapper<TextStyle>>,

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
			node.model.get().is_none(),
			"Internal: Node already has text attached!"
		);

		let client = node.get_client().ok_or_else(|| eyre!("Client not found"))?;
		let text = TEXT_REGISTRY.add(Text {
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
		let _ = node.text.set(text.clone());

		Ok(text)
	}

	fn draw(&self, sk: &StereoKitDraw) {
		let style = self.style.get_or_try_init(
			|| -> Result<SendWrapper<TextStyle>, color_eyre::eyre::Error> {
				let font = self
					.font_path
					.as_deref()
					.and_then(|path| Font::from_file(sk, path))
					.unwrap_or_else(|| Font::default(sk));
				Ok(SendWrapper::new(TextStyle::new(sk, font, 1.0, WHITE)))
			},
		);

		if let Ok(style) = style {
			let data = self.data.lock();
			let transform = self.space.global_transform()
				* Mat4::from_scale(vec3(
					data.character_height,
					data.character_height,
					data.character_height,
				));
			if let Some(bounds) = data.bounds {
				text::draw_in(
					sk,
					&data.text,
					transform,
					bounds / data.character_height,
					data.fit,
					style,
					data.bounds_align,
					data.text_align,
					vec3(0.0, 0.0, 0.0),
					Color128::from(data.color),
				);
			} else {
				text::draw_at(
					sk,
					&data.text,
					transform,
					style,
					data.bounds_align,
					data.text_align,
					vec3(0.0, 0.0, 0.0),
					data.color,
				);
			}
		}
	}

	pub fn set_character_height_flex(
		node: &Node,
		_calling_client: Arc<Client>,
		data: &[u8],
	) -> Result<()> {
		let height = flexbuffers::Reader::get_root(data)?.get_f64()? as f32;
		node.text.get().unwrap().data.lock().character_height = height;
		Ok(())
	}

	pub fn set_text_flex(node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		let text = flexbuffers::Reader::get_root(data)?.get_str()?.to_string();
		node.text.get().unwrap().data.lock().text = text;
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

pub fn draw_all(sk: &StereoKitDraw) {
	for text in TEXT_REGISTRY.get_valid_contents() {
		text.draw(sk);
	}
}

pub fn create_flex(_node: &Node, calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
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
	let info: CreateTextInfo = deserialize(data)?;
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

use crate::{
	core::{
		client::Client,
		destroy_queue,
		registry::Registry,
		resource::{parse_resource_id, ResourceID},
	},
	nodes::{
		spatial::{get_spatial_parent_flex, parse_transform, Spatial},
		Node,
	},
};
use anyhow::{anyhow, ensure, Result};
use glam::{vec3, Mat4, Vec2};
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use prisma::{FromTuple, Rgb, Rgba};
use send_wrapper::SendWrapper;
use stardust_xr::values::{parse_f32, parse_vec2};
use std::{convert::TryFrom, path::PathBuf, sync::Arc};
use stereokit::{
	font::Font,
	lifecycle::DrawContext,
	text::{self, TextAlign, TextFit, TextStyle},
	StereoKit,
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
		bounds: Option<Vec2>,
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

		let text = TEXT_REGISTRY.add(Text {
			space: node.spatial.get().unwrap().clone(),
			font_path: font_resource_id.and_then(|res| {
				res.get_file(&node.get_client().base_resource_prefixes.lock().clone())
			}),
			style: OnceCell::new(),

			data: Mutex::new(TextData {
				text,
				character_height,
				text_align,
				bounds,
				fit,
				bounds_align,
				color,
			}),
		});
		node.add_local_signal("setCharacterHeight", Text::set_character_height_flex);
		node.add_local_signal("setText", Text::set_text_flex);
		let _ = node.text.set(text.clone());

		Ok(text)
	}

	fn draw(&self, sk: &StereoKit, draw_ctx: &DrawContext) {
		let style =
			self.style
				.get_or_try_init(|| -> Result<SendWrapper<TextStyle>, anyhow::Error> {
					let font = if let Some(path) = self.font_path.as_deref() {
						Font::from_file(sk, path)
					} else {
						Some(Font::default(sk))
					};
					Ok(SendWrapper::new(TextStyle::new(
						sk,
						font.ok_or(std::fmt::Error)?,
						1.0,
						Rgba::new(Rgb::new(1.0, 1.0, 1.0), 1.0),
					)))
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
				text::draw_in(
					draw_ctx,
					&data.text,
					transform,
					bounds / data.character_height,
					data.fit,
					style,
					data.bounds_align,
					data.text_align,
					vec3(0.0, 0.0, 0.0),
					data.color,
				);
			} else {
				text::draw_at(
					draw_ctx,
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

pub fn draw_all(sk: &StereoKit, draw_ctx: &DrawContext) {
	for text in TEXT_REGISTRY.get_valid_contents() {
		text.draw(sk, draw_ctx);
	}
}

pub fn create_flex(_node: &Node, calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
	let flex_vec = flexbuffers::Reader::get_root(data)?.get_vector()?;
	let node = Node::create(
		&calling_client,
		"/drawable/text",
		flex_vec.idx(0).get_str()?,
		true,
	);
	let parent = get_spatial_parent_flex(&calling_client, flex_vec.index(1)?.get_str()?)?;
	let transform = parse_transform(flex_vec.index(2)?, true, true, true)?;
	let text = flex_vec.index(3)?.get_str()?.to_string();
	let font_resource_id = parse_resource_id(flex_vec.index(4)?).ok();
	let character_height = flex_vec.index(5)?.get_f64()? as f32;
	let text_align = TextAlign::from_bits(flex_vec.index(6)?.get_u64()? as u32)
		.ok_or_else(|| anyhow!("Text align bitflag out of range"))?;
	let bounds = parse_vec2(flex_vec.index(7)?).map(|bounds| bounds.into());
	let fit = TextFit::try_from(flex_vec.index(8)?.get_u64()? as u32)?;
	let bounds_align = TextAlign::from_bits(flex_vec.index(9)?.get_u64()? as u32)
		.ok_or_else(|| anyhow!("Bounds align bitflag out of range"))?;
	let color_vec = flex_vec.index(10)?.get_vector()?;
	let color = Rgba::from_tuple((
		(
			parse_f32(color_vec.index(0)?).ok_or_else(|| anyhow!("Value in color invalid"))?,
			parse_f32(color_vec.index(0)?).ok_or_else(|| anyhow!("Value in color invalid"))?,
			parse_f32(color_vec.index(0)?).ok_or_else(|| anyhow!("Value in color invalid"))?,
		),
		parse_f32(color_vec.index(0)?).ok_or_else(|| anyhow!("Value in color invalid"))?,
	));

	let node = node.add_to_scenegraph();
	Spatial::add_to(&node, Some(parent), transform)?;
	Text::add_to(
		&node,
		font_resource_id,
		text,
		character_height,
		text_align,
		bounds,
		fit,
		bounds_align,
		color,
	)?;
	Ok(())
}

use crate::{
	core::{client::Client, destroy_queue, registry::Registry, resource::get_resource_file},
	nodes::{spatial::Spatial, Node},
};
use color_eyre::eyre::{eyre, Result};
use glam::{vec3, Mat4, Vec2};
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use std::{ffi::OsStr, path::PathBuf, sync::Arc};
use stereokit_rust::{
	font::Font,
	sk::MainThreadToken,
	system::{TextAlign, TextFit, TextStyle as SkTextStyle},
	util::{Color128, Color32},
};

use super::{TextAspect, TextStyle};

static TEXT_REGISTRY: Registry<Text> = Registry::new();

fn convert_align(x_align: super::XAlign, y_align: super::YAlign) -> TextAlign {
	match (x_align, y_align) {
		(super::XAlign::Left, super::YAlign::Top) => TextAlign::TopLeft,
		(super::XAlign::Left, super::YAlign::Center) => TextAlign::CenterLeft,
		(super::XAlign::Left, super::YAlign::Bottom) => TextAlign::BottomLeft,
		(super::XAlign::Center, super::YAlign::Top) => TextAlign::Center,
		(super::XAlign::Center, super::YAlign::Center) => TextAlign::Center,
		(super::XAlign::Center, super::YAlign::Bottom) => TextAlign::BottomCenter,
		(super::XAlign::Right, super::YAlign::Top) => TextAlign::TopRight,
		(super::XAlign::Right, super::YAlign::Center) => TextAlign::CenterRight,
		(super::XAlign::Right, super::YAlign::Bottom) => TextAlign::BottomRight,
	}
}

pub struct Text {
	space: Arc<Spatial>,
	font_path: Option<PathBuf>,
	style: OnceCell<SkTextStyle>,

	text: Mutex<String>,
	data: Mutex<TextStyle>,
}
impl Text {
	pub fn add_to(node: &Arc<Node>, text: String, style: TextStyle) -> Result<Arc<Text>> {
		let client = node.get_client().ok_or_else(|| eyre!("Client not found"))?;
		let text = TEXT_REGISTRY.add(Text {
			space: node.get_aspect::<Spatial>().unwrap().clone(),
			font_path: style.font.as_ref().and_then(|res| {
				get_resource_file(res, &client, &[OsStr::new("ttf"), OsStr::new("otf")])
			}),
			style: OnceCell::new(),

			text: Mutex::new(text),
			data: Mutex::new(style),
		});
		node.add_aspect_raw(text.clone());

		Ok(text)
	}

	fn draw(&self, token: &MainThreadToken) {
		let style =
			self.style
				.get_or_try_init(|| -> Result<SkTextStyle, color_eyre::eyre::Error> {
					let font = self
						.font_path
						.as_deref()
						.and_then(|path| Font::from_file(path).ok())
						.unwrap_or_default();
					Ok(SkTextStyle::from_font(font, 1.0, Color32::WHITE))
				});

		if let Ok(style) = style {
			let text = self.text.lock();
			let data = self.data.lock();
			let transform = self.space.global_transform()
				* Mat4::from_scale(vec3(
					data.character_height,
					data.character_height,
					data.character_height,
				));
			if let Some(bounds) = &data.bounds {
				stereokit_rust::system::Text::add_in(
					token,
					&*text,
					transform,
					Vec2::from(bounds.bounds) / data.character_height,
					match bounds.fit {
						super::TextFit::Wrap => TextFit::Wrap,
						super::TextFit::Clip => TextFit::Clip,
						super::TextFit::Squeeze => TextFit::Squeeze,
						super::TextFit::Exact => TextFit::Exact,
						super::TextFit::Overflow => TextFit::Overflow,
					},
					Some(*style),
					Some(Color128::new(
						data.color.c.r,
						data.color.c.g,
						data.color.c.b,
						data.color.a,
					)),
					data.bounds
						.as_ref()
						.map(|b| convert_align(b.anchor_align_x, b.anchor_align_y)),
					Some(convert_align(data.text_align_x, data.text_align_y)),
					None,
					None,
					None,
				);
			} else {
				stereokit_rust::system::Text::add_at(
					token,
					&*text,
					transform,
					Some(*style),
					Some(Color128::new(
						data.color.c.r,
						data.color.c.g,
						data.color.c.b,
						data.color.a,
					)),
					data.bounds
						.as_ref()
						.map(|b| convert_align(b.anchor_align_x, b.anchor_align_y)),
					Some(convert_align(data.text_align_x, data.text_align_y)),
					None,
					None,
					None,
				);
			}
		}
	}
}
impl TextAspect for Text {
	fn set_character_height(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		height: f32,
	) -> Result<()> {
		let this_text = node.get_aspect::<Text>()?;
		this_text.data.lock().character_height = height;
		Ok(())
	}

	fn set_text(node: Arc<Node>, _calling_client: Arc<Client>, text: String) -> Result<()> {
		let this_text = node.get_aspect::<Text>()?;
		*this_text.text.lock() = text;
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

pub fn draw_all(token: &MainThreadToken) {
	for text in TEXT_REGISTRY.get_valid_contents() {
		if let Some(node) = text.space.node() {
			if node.enabled() {
				text.draw(token);
			}
		}
	}
}

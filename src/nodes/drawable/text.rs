use crate::{
	core::{client::Client, destroy_queue, registry::Registry, resource::get_resource_file},
	nodes::{spatial::Spatial, Aspect, Node},
};
use color_eyre::eyre::{eyre, Result};
use glam::{vec3, Mat4, Vec2};
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use portable_atomic::{AtomicBool, Ordering};
use std::{ffi::OsStr, path::PathBuf, sync::Arc};
use stereokit::{
	named_colors::WHITE, Color128, StereoKitDraw, TextAlign, TextFit, TextStyle as SkTextStyle,
};

use super::{TextAspect, TextStyle};

static TEXT_REGISTRY: Registry<Text> = Registry::new();

fn convert_align(x_align: super::XAlign, y_align: super::YAlign) -> TextAlign {
	let x_align = match x_align {
		super::XAlign::Left => TextAlign::XLeft,
		super::XAlign::Center => TextAlign::XCenter,
		super::XAlign::Right => TextAlign::XRight,
	} as u32;
	let y_align = match y_align {
		super::YAlign::Top => TextAlign::YTop,
		super::YAlign::Center => TextAlign::YCenter,
		super::YAlign::Bottom => TextAlign::YBottom,
	} as u32;

	unsafe { std::mem::transmute(x_align | y_align) }
}

pub struct Text {
	enabled: Arc<AtomicBool>,
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
			enabled: node.enabled.clone(),
			space: node.get_aspect::<Spatial>().unwrap().clone(),
			font_path: style.font.as_ref().and_then(|res| {
				get_resource_file(&res, &client, &[OsStr::new("ttf"), OsStr::new("otf")])
			}),
			style: OnceCell::new(),

			text: Mutex::new(text),
			data: Mutex::new(style),
		});
		<Text as TextAspect>::add_node_members(node);
		node.add_aspect_raw(text.clone());

		Ok(text)
	}

	fn draw(&self, sk: &impl StereoKitDraw) {
		let style =
			self.style
				.get_or_try_init(|| -> Result<SkTextStyle, color_eyre::eyre::Error> {
					let font = self
						.font_path
						.as_deref()
						.and_then(|path| sk.font_create(path).ok())
						.unwrap_or_else(|| sk.font_find("default/font").unwrap());
					Ok(unsafe { sk.text_make_style(font, 1.0, WHITE) })
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
				sk.text_add_in(
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
					*style,
					convert_align(bounds.anchor_align_x.clone(), bounds.anchor_align_y.clone()),
					convert_align(data.text_align_x.clone(), data.text_align_y.clone()),
					vec3(0.0, 0.0, 0.0),
					Color128::from([data.color.c.r, data.color.c.g, data.color.c.b, data.color.a]),
				);
			} else {
				sk.text_add_at(
					&*text,
					transform,
					*style,
					TextAlign::Center,
					convert_align(data.text_align_x.clone(), data.text_align_y.clone()),
					vec3(0.0, 0.0, 0.0),
					Color128::from([data.color.c.r, data.color.c.g, data.color.c.b, data.color.a]),
				);
			}
		}
	}
}
impl Aspect for Text {
	const NAME: &'static str = "Text";
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

pub fn draw_all(sk: &impl StereoKitDraw) {
	for text in TEXT_REGISTRY.get_valid_contents() {
		if text.enabled.load(Ordering::Relaxed) {
			text.draw(sk);
		}
	}
}

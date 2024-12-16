use crate::{
	bevy_plugin::convert_linear_rgba,
	core::{
		client::Client, destroy_queue, error::Result, registry::Registry,
		resource::get_resource_file,
	},
	nodes::{spatial::Spatial, Aspect, Node},
	DefaultMaterial,
};
use bevy::{
	app::{App, Plugin, PostUpdate, PreUpdate},
	asset::{AssetServer, Assets, RenderAssetUsages},
	color::Color,
	image::Image,
	pbr::MeshMaterial3d,
	prelude::{
		default, BuildChildren as _, Camera, Camera2d, ChildBuild as _, Commands, Deref, Entity,
		Mesh, Mesh3d, Plane3d, Query, Res, ResMut, Resource, Transform,
	},
	render::{
		camera::RenderTarget,
		render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages},
	},
	text::{cosmic_text::Align, JustifyText, TextColor, TextFont},
	ui::{AlignItems, BackgroundColor, FlexDirection, JustifyContent, TargetCamera, Val},
};
use color_eyre::eyre::eyre;
use glam::{vec3, Mat4, Vec2, Vec3};
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use std::{ffi::OsStr, path::PathBuf, sync::Arc};

use super::{TextAspect, TextStyle};

static TEXT_REGISTRY: Registry<Text> = Registry::new();

// fn convert_align(x_align: super::XAlign, y_align: super::YAlign) -> bevy::text::Text2d {
// 	match (x_align, y_align) {
// 		(super::XAlign::Left, super::YAlign::Top) => TextAlign::TopLeft,
// 		(super::XAlign::Left, super::YAlign::Center) => TextAlign::CenterLeft,
// 		(super::XAlign::Left, super::YAlign::Bottom) => TextAlign::BottomLeft,
// 		(super::XAlign::Center, super::YAlign::Top) => TextAlign::Center,
// 		(super::XAlign::Center, super::YAlign::Center) => TextAlign::Center,
// 		(super::XAlign::Center, super::YAlign::Bottom) => TextAlign::BottomCenter,
// 		(super::XAlign::Right, super::YAlign::Top) => TextAlign::TopRight,
// 		(super::XAlign::Right, super::YAlign::Center) => TextAlign::CenterRight,
// 		(super::XAlign::Right, super::YAlign::Bottom) => TextAlign::BottomRight,
// 	}
// }

pub struct StardustTextPlugin;
impl Plugin for StardustTextPlugin {
	fn build(&self, app: &mut App) {
		let (tx, rx) = crossbeam_channel::unbounded();
		SPAWN_TEXT_SENDER.set(tx);
		app.insert_resource(SpawnTextReader(rx));
		app.add_systems(PostUpdate, update_text);
		app.add_systems(PreUpdate, spawn_text);
	}
}

fn update_text(mut surface_query: Query<(&mut Transform)>) {
	for text in TEXT_REGISTRY.get_valid_contents() {
		let Some((mut transform)) = text
			.surface
			.get()
			.and_then(|v| surface_query.get_mut(*v).ok())
		else {
			continue;
		};
		let data = text.data.lock();

		*transform = Transform::from_matrix(
			text.space.global_transform()
				* Mat4::from_scale(vec3(
					data.character_height,
					data.character_height,
					data.character_height,
				)),
		);
	}
}

fn spawn_text(
	reader: Res<SpawnTextReader>,
	mut cmds: Commands,
	mut images: ResMut<Assets<Image>>,
	mut meshes: ResMut<Assets<Mesh>>,
	mut mats: ResMut<Assets<DefaultMaterial>>,
	asset_server: Res<AssetServer>,
) {
	for text in reader.try_iter() {
		let data = text.data.lock();
		let size = Extent3d {
			width: (512.0 * data.bounds.as_ref().map(|v| v.bounds.x).unwrap_or(1.0)).floor() as u32,
			height: (512.0 * data.bounds.as_ref().map(|v| v.bounds.y).unwrap_or(1.0)).floor()
				as u32,
			..default()
		};

		// This is the texture that will be rendered to.
		let mut image = Image::new_fill(
			size,
			TextureDimension::D2,
			&[0, 0, 0, 0],
			TextureFormat::Bgra8UnormSrgb,
			RenderAssetUsages::default(),
		);
		// You need to set these texture usage flags in order to use the image as a render target
		image.texture_descriptor.usage = TextureUsages::TEXTURE_BINDING
			| TextureUsages::COPY_DST
			| TextureUsages::RENDER_ATTACHMENT;

		let image_handle = images.add(image);

		let cam = cmds
			.spawn((
				Camera2d,
				Camera {
					target: RenderTarget::Image(image_handle.clone()),
					..default()
				},
			))
			.id();
		let font = text
			.font_path
			.as_ref()
			.map(|v| asset_server.load(v.as_path()));

		let ui_root = cmds
			.spawn((
				bevy::ui::Node {
					// Cover the whole image
					width: Val::Percent(100.),
					height: Val::Percent(100.),
					flex_direction: FlexDirection::Column,
					justify_content: JustifyContent::Center,
					align_items: AlignItems::Center,
					..default()
				},
				BackgroundColor(Color::NONE),
				TargetCamera(cam),
			))
			.with_children(|parent| {
				parent.spawn((
					bevy::prelude::Text::new(text.text.lock().as_str()),
					TextFont {
						font: font.unwrap_or_else(|| TextFont::default().font),
						font_size: 40.0,
						..default()
					},
					TextColor(convert_linear_rgba(data.color).into()),
				));
			})
			.id();
		let surface = cmds
			.spawn((
				Mesh3d(
					meshes.add(Plane3d::new(
						Vec3::NEG_Z,
						data.bounds
							.as_ref()
							.map(|v| v.bounds.into())
							.unwrap_or(Vec2::ZERO)
							* 0.5,
					)),
				),
				MeshMaterial3d(mats.add(DefaultMaterial {
					base_color_texture: Some(image_handle),
					unlit: true,
					..default()
				})),
			))
			.id();
		text.cam_entity.set(cam);
		text.ui_root.set(ui_root);
		text.surface.set(surface);
	}
}
static SPAWN_TEXT_SENDER: OnceCell<crossbeam_channel::Sender<Arc<Text>>> = OnceCell::new();
#[derive(Resource, Deref)]
struct SpawnTextReader(crossbeam_channel::Receiver<Arc<Text>>);

pub struct Text {
	space: Arc<Spatial>,
	font_path: Option<PathBuf>,
	text: Mutex<String>,
	data: Mutex<TextStyle>,
	cam_entity: OnceCell<Entity>,
	ui_root: OnceCell<Entity>,
	surface: OnceCell<Entity>,
}
impl Text {
	pub fn add_to(node: &Arc<Node>, text: String, style: TextStyle) -> Result<Arc<Text>> {
		let client = node.get_client().ok_or_else(|| eyre!("Client not found"))?;
		let text = TEXT_REGISTRY.add(Text {
			space: node.get_aspect::<Spatial>().unwrap().clone(),
			font_path: style.font.as_ref().and_then(|res| {
				get_resource_file(res, &client, &[OsStr::new("ttf"), OsStr::new("otf")])
			}),
			text: Mutex::new(text),
			data: Mutex::new(style),
			ui_root: OnceCell::new(),
			cam_entity: OnceCell::new(),
			surface: OnceCell::new(),
		});
		node.add_aspect_raw(text.clone());
		if let Some(sender) = SPAWN_TEXT_SENDER.get() {
			sender.send(text.clone());
		}

		Ok(text)
	}

	// fn draw(&self, token: &MainThreadToken) {
	// 	let style =
	// 		self.style
	// 			.get_or_try_init(|| -> Result<SkTextStyle, color_eyre::eyre::Error> {
	// 				let font = self
	// 					.font_path
	// 					.as_deref()
	// 					.and_then(|path| Font::from_file(path).ok())
	// 					.unwrap_or_default();
	// 				Ok(SkTextStyle::from_font(font, 1.0, Color32::WHITE))
	// 			});
	//
	// 	if let Ok(style) = style {
	// 		let text = self.text.lock();
	// 		let data = self.data.lock();
	// 		let transform = self.space.global_transform()
	// 			* Mat4::from_scale(vec3(
	// 				data.character_height,
	// 				data.character_height,
	// 				data.character_height,
	// 			));
	// 		if let Some(bounds) = &data.bounds {
	// 			stereokit_rust::system::Text::add_in(
	// 				token,
	// 				&*text,
	// 				transform,
	// 				Vec2::from(bounds.bounds) / data.character_height,
	// 				match bounds.fit {
	// 					super::TextFit::Wrap => TextFit::Wrap,
	// 					super::TextFit::Clip => TextFit::Clip,
	// 					super::TextFit::Squeeze => TextFit::Squeeze,
	// 					super::TextFit::Exact => TextFit::Exact,
	// 					super::TextFit::Overflow => TextFit::Overflow,
	// 				},
	// 				Some(*style),
	// 				Some(Color128::new(
	// 					data.color.c.r,
	// 					data.color.c.g,
	// 					data.color.c.b,
	// 					data.color.a,
	// 				)),
	// 				data.bounds
	// 					.as_ref()
	// 					.map(|b| convert_align(b.anchor_align_x, b.anchor_align_y)),
	// 				Some(convert_align(data.text_align_x, data.text_align_y)),
	// 				None,
	// 				None,
	// 				None,
	// 			);
	// 		} else {
	// 			stereokit_rust::system::Text::add_at(
	// 				token,
	// 				&*text,
	// 				transform,
	// 				Some(*style),
	// 				Some(Color128::new(
	// 					data.color.c.r,
	// 					data.color.c.g,
	// 					data.color.c.b,
	// 					data.color.a,
	// 				)),
	// 				data.bounds
	// 					.as_ref()
	// 					.map(|b| convert_align(b.anchor_align_x, b.anchor_align_y)),
	// 				Some(convert_align(data.text_align_x, data.text_align_y)),
	// 				None,
	// 				None,
	// 				None,
	// 			);
	// 		}
	// 	}
	// }
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
		TEXT_REGISTRY.remove(self);
	}
}

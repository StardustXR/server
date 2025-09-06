use crate::{
	BevyMaterial,
	core::{
		bevy_channel::{BevyChannel, BevyChannelReader},
		client::Client,
		color::ColorConvert,
		entity_handle::EntityHandle,
		error::Result,
		registry::Registry,
		resource::get_resource_file,
	},
	nodes::{
		Node,
		drawable::XAlign,
		spatial::{Spatial, SpatialNode},
	},
};
use bevy::{platform::collections::HashMap, prelude::*};
use bevy_mesh_text_3d::{
	Align, Attrs, MeshTextPlugin, Settings as FontSettings, generate_meshes,
	text_glyphs::TextGlyphs,
};
use color_eyre::eyre::eyre;
use core::f32;
use cosmic_text::Metrics;
use parking_lot::Mutex;
use std::{ffi::OsStr, mem, path::PathBuf, sync::Arc};

static SPAWN_TEXT: BevyChannel<Arc<Text>> = BevyChannel::new();

pub struct TextNodePlugin;

impl Plugin for TextNodePlugin {
	fn build(&self, app: &mut App) {
		// Text init stuff
		// 1.0 for font size in meters
		app.add_plugins(MeshTextPlugin::new(1.0));
		app.world_mut()
			.resource_mut::<FontSettings>()
			.font_system
			.db_mut()
			.load_system_fonts();

		SPAWN_TEXT.init(app);
		app.init_resource::<MaterialRegistry>();
		app.add_systems(Update, spawn_text);
	}
}

fn spawn_text(
	mut mpsc: ResMut<BevyChannelReader<Arc<Text>>>,
	mut cmds: Commands,
	mut font_settings: ResMut<FontSettings>,
	mut material_registry: ResMut<MaterialRegistry>,
	mut materials: ResMut<Assets<BevyMaterial>>,
	mut meshes: ResMut<Assets<Mesh>>,
	mut font_registry: Local<FontDatabaseRegistry>,
) {
	while let Some(text) = mpsc.read() {
		if let Some(entity) = text.entity.lock().take() {
			cmds.entity(*entity).despawn();
		}
		let style = text.data.lock();
		let old_db = text.font_path.clone().map(|p| {
			let db = font_registry.get(p);
			mem::swap(font_settings.font_system.db_mut(), db);
			db
		});
		let attrs = Attrs::new().weight(cosmic_text::Weight::BOLD);
		let alignment = Some(match style.text_align_x {
			super::XAlign::Left => Align::Right,
			super::XAlign::Center => Align::Center,
			super::XAlign::Right => Align::Left,
		});
		let text_string = text.text.lock().clone();
		let mut text_glyphs = TextGlyphs::new(
			Metrics {
				font_size: style.character_height,
				line_height: style.character_height,
			},
			[(text_string.as_str(), attrs.clone())],
			&attrs,
			&mut font_settings.font_system,
			alignment,
		);
		let max_width = style.bounds.as_ref().map(|v| v.bounds.x);
		let max_height = style.bounds.as_ref().map(|v| v.bounds.x);
		let (width, height) =
			text_glyphs.measure(max_width, max_height, &mut font_settings.font_system);
		let char_meshes = generate_meshes(
			bevy_mesh_text_3d::InputText::Simple {
				text: text_string,
				material: material_registry.get_handle(
					BevyMaterial {
						base_color: style.color.to_bevy(),
						emissive: Color::WHITE.to_linear(),
						metallic: 0.0,
						perceptual_roughness: 1.0,
						// If alpha is supported on text we need to change this
						alpha_mode: AlphaMode::Opaque,
						double_sided: false,
						..default()
					},
					&mut materials,
				),
				attrs,
			},
			&mut font_settings,
			bevy_mesh_text_3d::Parameters {
				extrusion_depth: 0.0,
				font_size: style.character_height,
				line_height: style.character_height,
				alignment,
				max_width,
				max_height,
			},
			&mut meshes,
		);
		if let Some(db) = old_db {
			mem::swap(font_settings.font_system.db_mut(), db);
		}
		let Ok(char_meshes) =
			char_meshes.inspect_err(|err| error!("unable to create text meshes: {err}"))
		else {
			continue;
		};
		// TODO: text align
		let letters = char_meshes
			.into_iter()
			.map(|v| {
				cmds.spawn((
					Mesh3d(v.mesh),
					MeshMaterial3d(v.material),
					Transform::from_xyz(
						// -dist +
						match style.text_align_x {
							XAlign::Left => 0.0,
							XAlign::Center => width * -0.5,
							XAlign::Right => -width,
						},
						match style.text_align_y {
							YAlign::Top => height,
							YAlign::Center => height * 0.5,
							YAlign::Bottom => 0.0,
						},
						0.0,
					) * v.transform,
				))
				.id()
			})
			.collect::<Vec<_>>();
		let entity = cmds
			.spawn((
				Name::new("TextNode"),
				SpatialNode(Arc::downgrade(&text.spatial)),
			))
			.add_children(&letters)
			.id();
		text.entity.lock().replace(EntityHandle(entity));
		text.spatial.set_entity(entity);
	}
}

#[derive(Default)]
struct FontDatabaseRegistry(HashMap<PathBuf, cosmic_text::fontdb::Database>);
impl FontDatabaseRegistry {
	fn get(&mut self, path: PathBuf) -> &mut cosmic_text::fontdb::Database {
		self.0.entry(path).or_insert_with_key(|path| {
			let mut db = cosmic_text::fontdb::Database::new();
			if let Err(err) = db.load_font_file(path) {
				error!("unable to load font file {} {err}", path.to_string_lossy());
			};
			db
		})
	}
}

use super::{TextAspect, TextStyle, YAlign, model::MaterialRegistry};

static TEXT_REGISTRY: Registry<Text> = Registry::new();

pub struct Text {
	spatial: Arc<Spatial>,
	font_path: Option<PathBuf>,
	entity: Mutex<Option<EntityHandle>>,
	text: Mutex<String>,
	data: Mutex<TextStyle>,
}
impl Text {
	pub fn add_to(node: &Arc<Node>, text: String, style: TextStyle) -> Result<Arc<Text>> {
		let client = node.get_client().ok_or_else(|| eyre!("Client not found"))?;
		let text = TEXT_REGISTRY.add(Text {
			spatial: node.get_aspect::<Spatial>().unwrap().clone(),
			font_path: style.font.as_ref().and_then(|res| {
				get_resource_file(res, &client, &[OsStr::new("ttf"), OsStr::new("otf")])
			}),

			entity: Mutex::new(None),
			text: Mutex::new(text),
			data: Mutex::new(style),
		});
		node.add_aspect_raw(text.clone());
		_ = SPAWN_TEXT.send(text.clone());

		Ok(text)
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
		_ = SPAWN_TEXT.send(this_text);
		Ok(())
	}

	fn set_text(node: Arc<Node>, _calling_client: Arc<Client>, text: String) -> Result<()> {
		let this_text = node.get_aspect::<Text>()?;
		*this_text.text.lock() = text;
		_ = SPAWN_TEXT.send(this_text);
		Ok(())
	}
}
impl Drop for Text {
	fn drop(&mut self) {
		TEXT_REGISTRY.remove(self);
	}
}

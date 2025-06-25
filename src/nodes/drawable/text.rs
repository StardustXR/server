use crate::{
	core::{
		client::Client, color::ColorConvert, error::Result, registry::Registry,
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
use bevy_sk::vr_materials::PbrMaterial;
use color_eyre::eyre::eyre;
use core::f32;
use cosmic_text::Metrics;
use parking_lot::Mutex;
use std::{
	ffi::OsStr,
	mem,
	path::PathBuf,
	sync::{Arc, OnceLock},
};
use tokio::sync::mpsc;

static SPAWN_TEXT: OnceLock<mpsc::UnboundedSender<Arc<Text>>> = OnceLock::new();

#[derive(Resource)]
struct MpscReceiver<T>(mpsc::UnboundedReceiver<T>);

pub struct TextNodePlugin;

impl Plugin for TextNodePlugin {
	fn build(&self, app: &mut App) {
		// Text init stuff
		// app.init_asset::<Font>().init_asset_loader::<FontLoader>();
		// load_internal_binary_asset!(
		// 	app,
		// 	Handle::default(),
		// 	"assets/FiraMono-subset.ttf",
		// 	|bytes: &[u8], _path: String| { Font::try_from_bytes(bytes.to_vec()).unwrap() }
		// );
		// 1.0 for font size in meters
		app.add_plugins(MeshTextPlugin::new(1.0));
		app.world_mut()
			.resource_mut::<FontSettings>()
			.font_system
			.db_mut()
			.load_system_fonts();

		let (tx, rx) = mpsc::unbounded_channel();
		SPAWN_TEXT.set(tx).unwrap();
		app.init_resource::<MaterialRegistry>();
		app.insert_resource(MpscReceiver(rx));
		app.add_systems(Update, (spawn_text, update_visibillity).chain());
	}
}

fn update_visibillity(mut cmds: Commands) {
	for text in TEXT_REGISTRY.get_valid_contents().into_iter() {
		let Some(entity) = text.entity.lock().as_ref().copied() else {
			continue;
		};
		match text.spatial.node().map(|n| n.enabled()).unwrap_or(false) {
			true => {
				cmds.entity(entity)
					.insert_recursive::<Children>(Visibility::Visible);
			}
			false => {
				cmds.entity(entity)
					.insert_recursive::<Children>(Visibility::Hidden);
			}
		}
	}
}

fn spawn_text(
	mut mpsc: ResMut<MpscReceiver<Arc<Text>>>,
	mut cmds: Commands,
	mut font_settings: ResMut<FontSettings>,
	mut material_registry: ResMut<MaterialRegistry>,
	mut materials: ResMut<Assets<PbrMaterial>>,
	mut meshes: ResMut<Assets<Mesh>>,
	mut font_registry: Local<FontDatabaseRegistry>,
) {
	while let Ok(text) = mpsc.0.try_recv() {
		if let Some(entity) = text.entity.lock().take() {
			cmds.entity(entity).despawn();
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
		info!(width, height, ?style.text_align_x);
		let meshes = generate_meshes(
			bevy_mesh_text_3d::InputText::Simple {
				text: text_string,
				material: material_registry.get_handle(
					PbrMaterial {
						color: style.color.to_bevy(),
						emission_factor: Color::WHITE,
						metallic: 0.0,
						roughness: 1.0,
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
		let Ok(meshes) = meshes.inspect_err(|err| error!("unable to create text meshes: {err}"))
		else {
			continue;
		};
		let dist = meshes.iter().fold(f32::MAX, |dist, v| {
			dist.min(v.transform.translation.x)
			// if dist > v.transform.translation.x {
			// 	v.transform.translation.x
			// } else {
			// 	dist
			// }
		});
		// TODO: text align
		let letters = meshes
			.into_iter()
			.map(|v| {
				// info!("{:?}", v.transform);
				cmds.spawn((
					Mesh3d(v.mesh),
					MeshMaterial3d(v.material),
					// rotation is sus, might be related to the gltf coordinate system
					Transform::from_rotation(Quat::from_rotation_y(f32::consts::PI))
						* Transform::from_xyz(
							-dist
								+ match style.bounds.as_ref().map(|v| v.anchor_align_x) {
									Some(XAlign::Center) => width * -0.5,
									Some(XAlign::Right) => width * -1.0,
									Some(XAlign::Left) => 0.0,
									None => 0.0,
								},
							0.0,
							0.0,
						) * v.transform,
				))
				.id()
			})
			.collect::<Vec<_>>();
		let entity = cmds
			.spawn((SpatialNode(Arc::downgrade(&text.spatial)),))
			.add_children(&letters)
			.id();
		text.entity.lock().replace(entity);
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

use super::{TextAspect, TextStyle, model::MaterialRegistry};

static TEXT_REGISTRY: Registry<Text> = Registry::new();

pub struct Text {
	spatial: Arc<Spatial>,
	font_path: Option<PathBuf>,
	entity: Mutex<Option<Entity>>,
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
		_ = SPAWN_TEXT.get().unwrap().send(text.clone());

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
		_ = SPAWN_TEXT.get().unwrap().send(this_text.clone());
		Ok(())
	}

	fn set_text(node: Arc<Node>, _calling_client: Arc<Client>, text: String) -> Result<()> {
		let this_text = node.get_aspect::<Text>()?;
		*this_text.text.lock() = text;
		_ = SPAWN_TEXT.get().unwrap().send(this_text.clone());
		Ok(())
	}
}
impl Drop for Text {
	fn drop(&mut self) {
		TEXT_REGISTRY.remove(self);
	}
}

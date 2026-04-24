use crate::{
	BevyMaterial, PION,
	bevy_int::{
		bevy_channel::{BevyChannel, BevyChannelReader},
		color::ColorConvert,
		entity_handle::EntityHandle,
	},
	core::resource::get_resource_file,
	interface,
	nodes::{
		ProxyExt as _,
		drawable::model::MaterialRegistry,
		spatial::{SpatialNode, SpatialObject},
	},
};
use bevy::{platform::collections::HashMap, prelude::*};
use bevy_mesh_text_3d::{
	Align, Attrs, HorizontalAnchorPoint, MeshTextPlugin, Settings as FontSettings, VerticalAlign,
	VerticalAnchorPoint, generate_meshes,
};
use binderbinder::binder_object::BinderObject;
use core::f32;
use gluon_wire::impl_transaction_handler;
use parking_lot::Mutex;
use stardust_xr_protocol::text::Text as TextProxy;
use stardust_xr_protocol::text::{
	TextFit, TextHandler, TextInterfaceHandler, TextStyle, XAlign, YAlign,
};
use std::{ffi::OsStr, mem, path::PathBuf, sync::Arc};

static SPAWN_TEXT: BevyChannel<Arc<Text>> = BevyChannel::new();

pub struct TextNodePlugin;

impl Plugin for TextNodePlugin {
	fn build(&self, app: &mut App) {
		// Text init stuff
		app.add_plugins(MeshTextPlugin);
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
			XAlign::Left => Align::Right,
			XAlign::Center => Align::Center,
			XAlign::Right => Align::Left,
		});
		let vertical_alignment = Some(match style.text_align_y {
			YAlign::Top => VerticalAlign::Top,
			YAlign::Center => VerticalAlign::Middle,
			YAlign::Bottom => VerticalAlign::Bottom,
		});
		let text_string = text.text.lock().clone();
		let max_width = style.bounds.as_ref().map(|v| v.bounds.x);
		let max_height = style.bounds.as_ref().map(|v| v.bounds.y);
		let horizontal_anchor_point = style
			.bounds
			.as_ref()
			.map(|v| match v.anchor_align_x {
				XAlign::Left => HorizontalAnchorPoint::Left,
				XAlign::Center => HorizontalAnchorPoint::Middle,
				XAlign::Right => HorizontalAnchorPoint::Right,
			})
			.unwrap_or(HorizontalAnchorPoint::Middle);
		let vertical_anchor_point = style
			.bounds
			.as_ref()
			.map(|v| match v.anchor_align_y {
				YAlign::Top => VerticalAnchorPoint::Top,
				YAlign::Center => VerticalAnchorPoint::Middle,
				YAlign::Bottom => VerticalAnchorPoint::Bottom,
			})
			.unwrap_or(VerticalAnchorPoint::Middle);
		let wrap = matches!(style.bounds.as_ref().map(|v| v.fit), Some(TextFit::Wrap));
		let char_meshes = generate_meshes(
			bevy_mesh_text_3d::InputText::Simple {
				text: text_string,
				material: material_registry.get_handle(
					BevyMaterial {
						base_color: style.color.to_bevy(),
						emissive: Color::WHITE.to_linear(),
						metallic: 0.0,
						perceptual_roughness: 1.0,
						alpha_mode: AlphaMode::Premultiplied,
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
				line_height: style.character_height * 1.1,
				alignment,
				max_width: wrap.then_some(0).and(max_width),
				max_height: wrap.then_some(0).and(max_height),
				vertical_alignment,
				horizontal_anchor_point,
				vertical_anchor_point,
			},
			&mut meshes,
		);
		if let Some(db) = old_db {
			mem::swap(font_settings.font_system.db_mut(), db);
		}
		let Ok((char_meshes, _text_size)) =
			char_meshes.inspect_err(|err| error!("unable to create text meshes: {err}"))
		else {
			continue;
		};

		let letters = char_meshes
			.into_iter()
			.map(|v| {
				cmds.spawn((Mesh3d(v.mesh), MeshMaterial3d(v.material), v.transform))
					.id()
			})
			.collect::<Vec<_>>();
		let entity = cmds
			.spawn((
				Name::new("TextNode"),
				SpatialNode(Arc::downgrade(&**text.spatial)),
			))
			.add_children(&letters)
			.id();
		let entity = EntityHandle::new(entity);
		text.entity.lock().replace(entity.clone());
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

#[derive(Debug)]
pub struct Text {
	spatial: Arc<SpatialObject>,
	font_path: Option<PathBuf>,
	entity: Mutex<Option<EntityHandle>>,
	text: Mutex<String>,
	data: Mutex<TextStyle>,
}
/// only exists so we can send an Arc<Text> into SPAWN_TEXT
#[derive(Debug)]
struct TextObject(Arc<Text>);
impl TextObject {
	pub fn new(
		spatial: Arc<SpatialObject>,
		text: String,
		style: TextStyle,
		prefixes: &[PathBuf],
	) -> BinderObject<TextObject> {
		let text = Arc::new(Text {
			spatial,
			font_path: style.font.as_ref().and_then(|res| {
				get_resource_file(res, prefixes, &[OsStr::new("ttf"), OsStr::new("otf")])
			}),

			entity: Mutex::new(None),
			text: Mutex::new(text),
			data: Mutex::new(style),
		});
		_ = SPAWN_TEXT.send(text.clone());
		let text = PION.register_object(TextObject(text));
		text
	}
}
impl TextHandler for TextObject {
	async fn set_character_height(&self, _ctx: gluon_wire::GluonCtx, height: f32) {
		self.0.data.lock().character_height = height;
		_ = SPAWN_TEXT.send(self.0.clone());
	}

	async fn set_text(&self, _ctx: gluon_wire::GluonCtx, text: String) {
		*self.0.text.lock() = text;
		_ = SPAWN_TEXT.send(self.0.clone());
	}
}
interface!(TextInterface);
impl TextInterfaceHandler for TextInterface {
	async fn create_text(
		&self,
		_ctx: gluon_wire::GluonCtx,
		spatial: stardust_xr_protocol::spatial::Spatial,
		text: String,
		style: TextStyle,
	) -> TextProxy {
		let Some(spatial) = spatial.owned() else {
			// TODO: replace with proper error returning
			panic!("invalid spatial in model loading");
		};
		let text = TextObject::new(spatial, text, style, self.base_prefixes());
		let proxy = TextProxy::from_handler(&text);
		text.to_service();
		proxy
	}
}
impl_transaction_handler!(TextObject);

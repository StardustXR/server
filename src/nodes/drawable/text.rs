use crate::{
	BevyMaterial,
	bevy_int::{color::ColorConvert, entity_handle::EntityHandle},
	core::{registry::Registry, resource::get_resource_file},
	interface,
	nodes::{
		ProxyExt as _,
		drawable::model::MaterialRegistry,
		fields::FieldObject,
		spatial::{SpatialNode, SpatialObject},
	},
	query::ServerQueryable,
};
use bevy::{asset::RenderAssetUsages, platform::collections::HashMap, prelude::*};
use bevy_mesh_text_3d::{
	Align, Attrs, HorizontalAnchorPoint, MeshTextPlugin, Settings as FontSettings, VerticalAlign,
	VerticalAnchorPoint, generate_meshes,
};
use core::f32;
use gluon_ipc::{Handler, Interface, LocalRef, RefExt, ToRef};
use parking_lot::Mutex;
use stardust_xr_molecules_protocols::legible::{Legible, LegibleHandler};
use stardust_xr_protocol::{
	field::Shape,
	spatial::Spatial,
	text::{Text as TextProxy, TextLocal},
};
use stardust_xr_protocol::{
	text::{TextFit, TextHandler, TextInterfaceHandler, TextStyle, XAlign, YAlign},
	types::ResourceLoadError,
};
use stardust_xr_server_wboit::WboitMaterial;
use std::{
	ffi::OsStr,
	mem,
	path::PathBuf,
	sync::{
		Arc, Weak,
		atomic::{AtomicBool, Ordering},
	},
};

static TEXT_REGISTRY: Registry<Text> = Registry::new();

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

		app.init_resource::<MaterialRegistry>();
		app.add_systems(Update, spawn_text);
	}
}

fn spawn_text(
	mut cmds: Commands,
	mut font_settings: ResMut<FontSettings>,
	mut material_registry: ResMut<MaterialRegistry>,
	mut materials: ResMut<Assets<WboitMaterial>>,
	mut meshes: ResMut<Assets<Mesh>>,
	mut font_registry: Local<FontDatabaseRegistry>,
) {
	for text in TEXT_REGISTRY.get_valid_contents() {
		if !text.dirty.swap(false, Ordering::Relaxed) {
			continue;
		}
		let Some(spatial_entity) = text.spatial.get_entity() else {
			// the spatial hasn't been given an entity yet, try again next frame
			text.dirty.store(true, Ordering::Relaxed);
			continue;
		};
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
		if let Some(legibility) = text.legibility.lock().as_ref() {
			legibility.field.set_shape(text_box(&text_string, &style));
		}
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

		// one mesh per material rather than an entity per glyph, the renderer's per frame work
		// grows with entity count and a label is often dozens of glyphs
		let mut merged: Vec<(Handle<WboitMaterial>, Mesh)> = Vec::new();
		let mut loose = Vec::new();
		for glyph in char_meshes {
			let Some(mesh) = meshes.get(&glyph.mesh) else {
				continue;
			};
			let mesh = mesh.clone().transformed_by(glyph.transform);
			match merged.iter_mut().find(|(m, _)| *m == glyph.material) {
				Some((_, into)) => {
					if into.merge(&mesh).is_err() {
						loose.push((glyph.material, mesh));
					}
				}
				None => merged.push((glyph.material, mesh)),
			}
		}
		let letters = merged
			.into_iter()
			.chain(loose)
			.map(|(material, mut mesh)| {
				// nothing reads it back on the cpu, so don't keep a copy there
				mesh.asset_usage = RenderAssetUsages::RENDER_WORLD;
				cmds.spawn((
					Name::new("TextMesh"),
					Mesh3d(meshes.add(mesh)),
					MeshMaterial3d(material),
				))
				.id()
			})
			.collect::<Vec<_>>();
		let entity = cmds
			.spawn((
				ChildOf(spatial_entity),
				Name::new("Text"),
				SpatialNode(Arc::downgrade(&**text.spatial)),
			))
			.add_children(&letters)
			.id();
		text.entity.lock().replace(EntityHandle::new(entity));
	}
}

/// roughly how wide a character is for its height, a guess on purpose so nothing gets measured
const CHARACTER_ASPECT: f32 = 0.6;

// sized from what the client chose rather than the rendered glyphs, so the field can't be used
// to measure fonts, and anchored the same way the text layout anchors its block
fn text_box(text: &str, style: &TextStyle) -> Shape {
	let h = style.character_height;
	let (w, height, x, y) = match &style.bounds {
		Some(b) => (b.bounds.x, b.bounds.y, b.anchor_align_x, b.anchor_align_y),
		None => {
			let longest = text.lines().map(|l| l.chars().count()).max().unwrap_or(0);
			let lines = text.lines().count().max(1);
			(
				longest as f32 * h * CHARACTER_ASPECT,
				lines as f32 * h * 1.1,
				XAlign::Center,
				YAlign::Center,
			)
		}
	};
	let center = vec3(
		match x {
			XAlign::Left => w / 2.0,
			XAlign::Center => 0.0,
			XAlign::Right => -w / 2.0,
		},
		match y {
			YAlign::Top => -height / 2.0,
			YAlign::Center => 0.0,
			YAlign::Bottom => height / 2.0,
		},
		0.0,
	);
	Shape::Transform {
		shape: Box::new(Shape::Box {
			size: [w, height, 0.005].into(),
		}),
		transform: Mat4::from_translation(center).into(),
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

#[derive(Debug, Handler)]
pub struct Text {
	spatial: Arc<SpatialObject>,
	font_path: Option<PathBuf>,
	entity: Mutex<Option<EntityHandle>>,
	text: Mutex<String>,
	data: Mutex<TextStyle>,
	/// set by the handler methods, consumed by `spawn_text`, which rebuilds the meshes
	dirty: AtomicBool,
	legibility: Mutex<Option<Legibility>>,
}
#[derive(Debug)]
struct Legibility {
	field: Arc<FieldObject>,
	_queryable: ServerQueryable,
}
impl Text {
	pub fn new(
		spatial: Arc<SpatialObject>,
		text: String,
		style: TextStyle,
		prefixes: &[PathBuf],
	) -> TextLocal<Text> {
		let text = Arc::new(Text {
			spatial,
			font_path: style.font.as_ref().and_then(|res| {
				get_resource_file(res, prefixes, &[OsStr::new("ttf"), OsStr::new("otf")])
			}),

			entity: Mutex::new(None),
			text: Mutex::new(text),
			data: Mutex::new(style),
			dirty: AtomicBool::new(true),
			legibility: Mutex::new(None),
		});
		TEXT_REGISTRY.add_raw(&text);

		// TODO: remove this unwrap
		TextProxy::new_service(text).unwrap()
	}

	async fn make_legible(self: &Arc<Self>, spatial: LocalRef<Spatial, SpatialObject>) {
		let field = FieldObject::new(
			self.spatial.clone(),
			Shape::Box {
				size: [0.0; 3].into(),
			},
		);
		// only a weak ref, the queryable holds this service alive and the text holds the queryable
		let legible = match Legible::new_service(LegibleText(Arc::downgrade(self))) {
			Ok(legible) => legible,
			Err(err) => {
				error!("unable to make text legible: {err}");
				return;
			}
		};
		let queryable = ServerQueryable::new(
			spatial,
			field.clone(),
			[(<Legible as Interface>::ID, legible.proxy().to_ref())],
		)
		.await;
		self.legibility.lock().replace(Legibility {
			field: field.handler().clone(),
			_queryable: queryable,
		});
		self.dirty.store(true, Ordering::Relaxed);
	}
}

#[derive(Debug, Handler)]
struct LegibleText(Weak<Text>);
impl LegibleHandler for LegibleText {
	async fn text(&self, _ctx: gluon_ipc::Context) -> String {
		self.0
			.upgrade()
			.map(|t| t.text.lock().clone())
			.unwrap_or_default()
	}
}
impl TextHandler for Text {
	async fn set_character_height(&self, _ctx: gluon_ipc::Context, height: f32) {
		self.data.lock().character_height = height;
		self.dirty.store(true, Ordering::Relaxed);
	}

	async fn set_text(&self, _ctx: gluon_ipc::Context, text: String) {
		*self.text.lock() = text;
		self.dirty.store(true, Ordering::Relaxed);
	}
}
interface!(TextInterface);
impl TextInterfaceHandler for TextInterface {
	async fn create_text(
		&self,
		_ctx: gluon_ipc::Context,
		spatial: Spatial,
		text: String,
		style: TextStyle,
	) -> Result<TextProxy, ResourceLoadError> {
		let spatial = spatial.owned_ref().ok_or(ResourceLoadError::InvalidRef)?;
		info!(?text, "creating text");
		let text = Text::new(spatial.handler().clone(), text, style, self.base_prefixes());
		text.handler().make_legible(spatial).await;
		Ok(text.into_proxy())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use stardust_xr_protocol::{text::TextBounds, types::rgba_linear};

	fn style(bounds: Option<TextBounds>) -> TextStyle {
		TextStyle {
			character_height: 0.1,
			color: rgba_linear!(1.0, 1.0, 1.0, 1.0),
			text_align_x: XAlign::Left,
			text_align_y: YAlign::Top,
			font: None,
			bounds,
		}
	}
	fn size_and_center(shape: Shape) -> (Vec3, Vec3) {
		let Shape::Transform { shape, transform } = shape else {
			panic!("expected a transformed box");
		};
		let Shape::Box { size } = *shape else {
			panic!("expected a box");
		};
		(size.into(), Mat4::from(transform).w_axis.truncate())
	}

	#[test]
	fn unbounded_text_is_centered_and_sized_from_characters() {
		let (size, center) = size_and_center(text_box("hello\nhi", &style(None)));
		assert!(size.abs_diff_eq(vec3(0.3, 0.22, 0.005), 1e-5), "{size}");
		assert!(center.abs_diff_eq(Vec3::ZERO, 1e-5), "{center}");
	}

	#[test]
	fn same_length_text_gets_the_same_box() {
		// wide and narrow glyphs would measure differently, the field must not
		let a = size_and_center(text_box("WWWWW", &style(None)));
		let b = size_and_center(text_box("iiiii", &style(None)));
		assert_eq!(a, b);
	}

	#[test]
	fn bounded_text_uses_the_bounds_and_their_anchor() {
		let bounds = |x, y| {
			Some(TextBounds {
				bounds: [0.4, 0.2].into(),
				fit: TextFit::Wrap,
				anchor_align_x: x,
				anchor_align_y: y,
			})
		};
		for (x, y, expected) in [
			(XAlign::Left, YAlign::Top, vec3(0.2, -0.1, 0.0)),
			(XAlign::Center, YAlign::Center, Vec3::ZERO),
			(XAlign::Right, YAlign::Bottom, vec3(-0.2, 0.1, 0.0)),
		] {
			let (size, center) = size_and_center(text_box("anything at all", &style(bounds(x, y))));
			assert!(size.abs_diff_eq(vec3(0.4, 0.2, 0.005), 1e-5), "{size}");
			assert!(center.abs_diff_eq(expected, 1e-5), "{center} != {expected}");
		}
	}

	#[test]
	fn empty_text_has_no_width() {
		let (size, _) = size_and_center(text_box("", &style(None)));
		assert_eq!(size.x, 0.0);
	}
}

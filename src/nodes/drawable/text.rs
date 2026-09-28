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
use bevy::{platform::collections::HashMap, prelude::*, render::primitives::Aabb};
use bevy_rich_text3d::{
	Text3d, Text3dBounds, Text3dDimensionOut, Text3dPlugin, Text3dSet, Text3dStyling, TextAlign,
	TextAnchor, TextAtlas, TextRenderer, TouchTextMaterial3dPlugin, Weight,
};
use cosmic_text::fontdb::Source;
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
	path::{Path, PathBuf},
	sync::{
		Arc, Weak,
		atomic::{AtomicBool, Ordering},
	},
};

static TEXT_REGISTRY: Registry<Text> = Registry::new();

/// glyphs are rasterized at this many pixels per em, `world_scale` shrinks them back to meters
const GLYPH_PX: f32 = 64.0;

pub struct TextNodePlugin;
impl Plugin for TextNodePlugin {
	fn build(&self, app: &mut App) {
		app.add_plugins((
			Text3dPlugin {
				default_atlas_dimension: (1024, 1024),
				sync_scale_factor_with_main_window: false,
				load_system_fonts: true,
				..default()
			},
			TouchTextMaterial3dPlugin::<WboitMaterial>::default(),
		));
		app.init_resource::<MaterialRegistry>();
		app.add_systems(Update, update_text);
		app.add_systems(PostUpdate, rebound_text.after(Text3dSet));
	}
}

fn update_text(
	mut cmds: Commands,
	renderer: Option<ResMut<TextRenderer>>,
	mut material_registry: ResMut<MaterialRegistry>,
	mut materials: ResMut<Assets<WboitMaterial>>,
	mut families: Local<HashMap<PathBuf, Option<Arc<str>>>>,
) {
	let Some(mut renderer) = renderer else {
		return;
	};
	for text in TEXT_REGISTRY.get_valid_contents() {
		if !text.dirty.swap(false, Ordering::Relaxed) {
			continue;
		}
		let Some(spatial_entity) = text.spatial.get_entity() else {
			// the spatial hasn't been given an entity yet, try again next frame
			text.dirty.store(true, Ordering::Relaxed);
			continue;
		};
		let style = text.data.lock();
		let text_string = text.text.lock().clone();
		if let Some(legibility) = text.legibility.lock().as_ref() {
			legibility.field.set_shape(text_box(&text_string, &style));
		}
		let font = text
			.font_path
			.as_ref()
			.and_then(|p| {
				families
					.entry(p.clone())
					.or_insert_with_key(|p| load_family(&mut renderer, p))
					.clone()
			})
			.unwrap_or_else(|| "sans-serif".into());

		let h = style.character_height;
		let a = align(style.text_align_x, style.text_align_y);
		let (offset, width) = match &style.bounds {
			Some(b) => {
				let size = Vec2::from(b.bounds);
				(
					(a - align(b.anchor_align_x, b.anchor_align_y)) * size / 2.0,
					match b.fit {
						TextFit::Wrap => size.x / h * GLYPH_PX,
						_ => f32::MAX,
					},
				)
			}
			None => (Vec2::ZERO, f32::MAX),
		};
		let components = (
			Text3d::new(text_string),
			Text3dStyling {
				size: GLYPH_PX,
				font,
				weight: Weight::BOLD,
				align: match style.text_align_x {
					XAlign::Left => TextAlign::Left,
					XAlign::Center => TextAlign::Center,
					XAlign::Right => TextAlign::Right,
				},
				anchor: TextAnchor(-a / 2.0),
				line_height: 1.1,
				color: style.color.to_bevy().to_srgba(),
				world_scale: Some(Vec2::splat(h)),
				..default()
			},
			Text3dBounds { width },
			Transform::from_translation(offset.extend(0.0)),
		);

		let mut entity = text.entity.lock();
		match entity.as_ref() {
			Some(e) => {
				cmds.entity(**e).try_insert(components);
			}
			None => {
				// color comes in through the vertex colors, so every text shares this one
				let material = material_registry.get_handle(
					BevyMaterial {
						base_color_texture: Some(TextAtlas::DEFAULT_IMAGE),
						emissive: Color::WHITE.to_linear(),
						metallic: 0.0,
						perceptual_roughness: 1.0,
						alpha_mode: AlphaMode::Blend,
						double_sided: false,
						..default()
					},
					&mut materials,
				);
				let e = cmds
					.spawn((
						ChildOf(spatial_entity),
						Name::new("Text"),
						SpatialNode(Arc::downgrade(&**text.spatial)),
						Mesh3d::default(),
						MeshMaterial3d(material),
						components,
					))
					.id();
				entity.replace(EntityHandle::new(e));
			}
		}
	}
}

fn load_family(renderer: &mut TextRenderer, path: &Path) -> Option<Arc<str>> {
	let mut fonts = renderer.lock();
	let db = fonts.db_mut();
	let family = db
		.load_font_source(Source::File(path.to_path_buf()))
		.first()
		.and_then(|id| db.face(*id))
		.and_then(|f| f.families.first())
		.map(|(name, _)| name.as_str().into());
	if family.is_none() {
		error!("unable to load font file {}", path.to_string_lossy());
	}
	family
}

// text meshes are rewritten in place, and bevy only computes an Aabb when there isn't one
fn rebound_text(mut cmds: Commands, query: Query<Entity, Changed<Text3dDimensionOut>>) {
	for e in &query {
		cmds.entity(e).remove::<Aabb>();
	}
}

/// +X right and +Y up, the direction a block sits from its anchor
fn align(x: XAlign, y: YAlign) -> Vec2 {
	vec2(
		match x {
			XAlign::Left => -1.0,
			XAlign::Center => 0.0,
			XAlign::Right => 1.0,
		},
		match y {
			YAlign::Top => 1.0,
			YAlign::Center => 0.0,
			YAlign::Bottom => -1.0,
		},
	)
}

/// roughly how wide a character is for its height, a guess on purpose so nothing gets measured
const CHARACTER_ASPECT: f32 = 0.6;

// sized from what the client chose rather than the rendered glyphs, so the field can't be used
// to measure fonts, and anchored the same way the text layout anchors its block
fn text_box(text: &str, style: &TextStyle) -> Shape {
	let h = style.character_height;
	let (size, x, y) = match &style.bounds {
		Some(b) => (Vec2::from(b.bounds), b.anchor_align_x, b.anchor_align_y),
		None => {
			let longest = text.lines().map(|l| l.chars().count()).max().unwrap_or(0);
			let lines = text.lines().count().max(1);
			(
				vec2(
					longest as f32 * h * CHARACTER_ASPECT,
					lines as f32 * h * 1.1,
				),
				style.text_align_x,
				style.text_align_y,
			)
		}
	};
	let center = -align(x, y) * size / 2.0;
	Shape::Transform {
		shape: Box::new(Shape::Box {
			size: [size.x, size.y, 0.005].into(),
		}),
		transform: Mat4::from_translation(center.extend(0.0)).into(),
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
	fn unbounded_text_is_anchored_by_its_alignment_and_sized_from_characters() {
		let (size, center) = size_and_center(text_box("hello\nhi", &style(None)));
		assert!(size.abs_diff_eq(vec3(0.3, 0.22, 0.005), 1e-5), "{size}");
		assert!(center.abs_diff_eq(vec3(0.15, -0.11, 0.0), 1e-5), "{center}");
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

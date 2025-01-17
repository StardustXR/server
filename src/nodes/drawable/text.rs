use crate::{
	bevy_plugin::{convert_linear_rgba, DESTROY_ENTITY},
	core::{
		client::Client,
		error::{Result, ServerError},
		registry::Registry,
		resource::get_resource_file,
	},
	nodes::{spatial::Spatial, Node},
	DefaultMaterial,
};
use bevy::{
	app::{App, Plugin, PostUpdate, PreUpdate},
	asset::{AssetServer, Assets},
	pbr::MeshMaterial3d,
	prelude::{Commands, Deref, Entity, Query, Res, ResMut, Resource, Transform},
};
use bevy_mod_meshtext::{HorizontalLayout, MeshText, MeshTextFont, VerticalLayout};
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use std::{ffi::OsStr, path::PathBuf, sync::Arc};
use tracing::info_span;

use super::{TextAspect, TextStyle};

static TEXT_REGISTRY: Registry<Text> = Registry::new();

const fn convert_align_x(x_align: super::XAlign) -> HorizontalLayout {
	match x_align {
		super::XAlign::Left => HorizontalLayout::Left,
		super::XAlign::Center => HorizontalLayout::Centered,
		super::XAlign::Right => HorizontalLayout::Right,
	}
}
const fn convert_align_y(y_align: super::YAlign) -> VerticalLayout {
	match y_align {
		super::YAlign::Top => VerticalLayout::Top,
		super::YAlign::Center => VerticalLayout::Centered,
		super::YAlign::Bottom => VerticalLayout::Bottom,
	}
}

pub struct StardustTextPlugin;
impl Plugin for StardustTextPlugin {
	fn build(&self, app: &mut App) {
		let (tx, rx) = crossbeam_channel::unbounded();
		_ = SPAWN_TEXT_SENDER.set(tx);
		app.insert_resource(SpawnTextReader(rx));
		app.add_systems(PostUpdate, update_text);
		app.add_systems(PreUpdate, spawn_text);
	}
}

fn update_text(mut surface_query: Query<(&mut Transform)>) {
	for text in TEXT_REGISTRY.get_valid_contents() {
		let Some((mut transform)) = text
			.entity
			.get()
			.and_then(|v| surface_query.get_mut(*v).ok())
		else {
			continue;
		};
		// let data = text.data.lock();

		*transform = Transform::from_matrix(text.space.global_transform());
	}
}

fn spawn_text(
	reader: Res<SpawnTextReader>,
	mut cmds: Commands,
	mut mats: ResMut<Assets<DefaultMaterial>>,
	asset_server: Res<AssetServer>,
) {
	for text in reader.try_iter() {
		let _span = info_span!("spawning text").entered();
		let _span2 = info_span!("text data lock").entered();
		let data = text.data.lock();
		drop(_span2);
		let _span2 = info_span!("text str lock").entered();
		let str = text.text.lock().clone();
		drop(_span2);
		let mat = mats.add(DefaultMaterial {
			color: convert_linear_rgba(data.color).into(),
			..Default::default()
		});
		let font = text
			.font_path
			.as_ref()
			.map(|p| asset_server.load(p.as_path()));
		let mut text_entity = cmds.spawn((
			MeshText {
				text: atomicow::CowArc::Owned(str),
				height: data.character_height,
				depth: 0.0,
			},
			MeshMaterial3d(mat),
			convert_align_x(data.text_align_x),
			convert_align_y(data.text_align_y),
		));
		if let Some(font) = font {
			text_entity.insert(MeshTextFont(font));
		}

		let entity = text_entity.id();

		let _span = info_span!("setting OneCells").entered();
		text.entity.set(entity);
	}
}
static SPAWN_TEXT_SENDER: OnceCell<crossbeam_channel::Sender<Arc<Text>>> = OnceCell::new();
#[derive(Resource, Deref)]
struct SpawnTextReader(crossbeam_channel::Receiver<Arc<Text>>);

pub struct Text {
	space: Arc<Spatial>,
	font_path: Option<PathBuf>,
	text: Mutex<Arc<str>>,
	data: Mutex<TextStyle>,
	entity: OnceCell<Entity>,
}
impl Text {
	pub fn add_to(node: &Arc<Node>, text: String, style: TextStyle) -> Result<Arc<Text>> {
		let client = node.get_client().ok_or(ServerError::NoClient)?;
		let text = TEXT_REGISTRY.add(Text {
			space: node.get_aspect::<Spatial>().unwrap().clone(),
			font_path: style.font.as_ref().and_then(|res| {
				get_resource_file(res, &client, &[OsStr::new("ttf"), OsStr::new("otf")])
			}),
			text: Mutex::new(text.into()),
			data: Mutex::new(style),
			entity: OnceCell::new(),
		});
		node.add_aspect_raw(text.clone());
		if let Some(sender) = SPAWN_TEXT_SENDER.get() {
			sender.send(text.clone());
		}

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
		Ok(())
	}

	fn set_text(node: Arc<Node>, _calling_client: Arc<Client>, text: String) -> Result<()> {
		let this_text = node.get_aspect::<Text>()?;
		*this_text.text.lock() = text.into();
		Ok(())
	}
}
impl Drop for Text {
	fn drop(&mut self) {
		if let Some(e) = self.entity.get() {
			DESTROY_ENTITY.send(*e);
		}
		TEXT_REGISTRY.remove(self);
	}
}

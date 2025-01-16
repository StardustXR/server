use super::{Aspect, AspectIdentifier, Node};
use crate::core::client::Client;
use crate::core::destroy_queue;
use crate::core::error::{Result, ServerError};
use crate::core::registry::Registry;
use crate::core::resource::get_resource_file;
use crate::nodes::spatial::{Spatial, Transform, SPATIAL_ASPECT_ALIAS_INFO};
use bevy::app::{App, Plugin, PostUpdate, PreUpdate};
use bevy::asset::AssetServer;
use bevy::audio::{AudioPlayer, AudioSink, AudioSinkPlayback, PlaybackSettings, Volume};
use bevy::prelude::{Commands, Deref, Entity, Query, Res, Resource, Transform as BevyTransform};
use color_eyre::eyre::eyre;
use once_cell::sync::OnceCell;
use stardust_xr::values::ResourceID;
use tracing::error;

use std::sync::Arc;
use std::{ffi::OsStr, path::PathBuf};

static SOUND_REGISTRY: Registry<Sound> = Registry::new();

pub struct StardustSoundPlugin;
impl Plugin for StardustSoundPlugin {
	fn build(&self, app: &mut App) {
		let (tx, rx) = crossbeam_channel::unbounded();
		_ = SOUND_EVENT_SENDER.set(tx);
		app.insert_resource(SoundEventReader(rx));
		let (tx, rx) = crossbeam_channel::unbounded();
		_ = SPAWN_SOUND_SENDER.set(tx);
		app.insert_resource(SpawnSoundReader(rx));
		app.add_systems(PostUpdate, update_sound_state);
		app.add_systems(PreUpdate, spawn_sounds);
	}
}

fn spawn_sounds(reader: Res<SpawnSoundReader>, mut cmds: Commands, asset_server: Res<AssetServer>) {
	for sound in reader.try_iter() {
		let e = cmds
			.spawn((
				BevyTransform::default(),
				AudioPlayer::new(asset_server.load(sound.pending_audio_path.as_path())),
				PlaybackSettings {
					mode: bevy::audio::PlaybackMode::Once,
					volume: Volume::new(sound.volume),
					speed: 1.0,
					paused: true,
					spatial: true,
					spatial_scale: None,
				},
			))
			.id();
		let _ = sound.entity.set(e);
	}
}

fn update_sound_state(
	mut query: Query<&mut AudioSink>,
	mut transforms: Query<&mut BevyTransform>,
	mut cmds: Commands,
	reader: Res<SoundEventReader>,
) {
	for (entity, action) in reader.try_iter() {
		match action {
			SoundAction::Stop => {
				let Ok(sink) = query.get_mut(entity) else {
					error!("no audio sink to stop?");
					continue;
				};
				sink.stop()
			}
			SoundAction::Play => {
				// Idk let's hope this works?
				cmds.entity(entity).remove::<AudioSink>();
			}
		}
	}
	for sound in SOUND_REGISTRY.get_valid_contents() {
		let Some(entity) = sound.entity.get().copied() else {
			continue;
		};
		let Ok(mut transform) = transforms.get_mut(entity) else {
			continue;
		};
		*transform = BevyTransform::from_matrix(sound.space.global_transform());
	}
}

stardust_xr_server_codegen::codegen_audio_protocol!();
pub struct Sound {
	space: Arc<Spatial>,
	volume: f32,
	pending_audio_path: PathBuf,
	entity: OnceCell<Entity>,
}
static SPAWN_SOUND_SENDER: OnceCell<crossbeam_channel::Sender<Arc<Sound>>> = OnceCell::new();
#[derive(Resource, Deref)]
struct SpawnSoundReader(crossbeam_channel::Receiver<Arc<Sound>>);
static SOUND_EVENT_SENDER: OnceCell<crossbeam_channel::Sender<(Entity, SoundAction)>> =
	OnceCell::new();
#[derive(Resource, Deref)]
struct SoundEventReader(crossbeam_channel::Receiver<(Entity, SoundAction)>);
pub enum SoundAction {
	// Pause and Resume?
	Stop,
	Play,
}
impl Sound {
	pub fn add_to(node: &Arc<Node>, resource_id: ResourceID) -> Result<Arc<Sound>> {
		let pending_audio_path = get_resource_file(
			&resource_id,
			&*node.get_client().ok_or(ServerError::NoClient)?,
			&[OsStr::new("wav"), OsStr::new("mp3")],
		)
		.ok_or(ServerError::NoResource)?;
		let sound = Sound {
			space: node.get_aspect::<Spatial>().unwrap().clone(),
			volume: 1.0,
			pending_audio_path,
			entity: OnceCell::new(),
		};
		let sound_arc = SOUND_REGISTRY.add(sound);
		node.add_aspect_raw(sound_arc.clone());
		if let Some(sender) = SPAWN_SOUND_SENDER.get() {
			sender
				.send(sound_arc.clone())
				.map_err(|_| eyre!("Unable to Spawn Audio Node"))?;
		}
		Ok(sound_arc)
	}
}
impl AspectIdentifier for Sound {
	impl_aspect_for_sound_aspect_id! {}
}
impl Aspect for Sound {
	impl_aspect_for_sound_aspect! {}
}
impl SoundAspect for Sound {
	fn play(node: Arc<Node>, _calling_client: Arc<Client>) -> Result<()> {
		let sound = node.get_aspect::<Sound>().unwrap();
		if let Some((sender, entity)) = SOUND_EVENT_SENDER
			.get()
			.and_then(|s| Some((s, *sound.entity.get()?)))
		{
			sender
				.send((entity, SoundAction::Play))
				.map_err(|_| eyre!("Unable to Play Audio"))?
		}
		Ok(())
	}
	fn stop(node: Arc<Node>, _calling_client: Arc<Client>) -> Result<()> {
		let sound = node.get_aspect::<Sound>().unwrap();
		if let Some((sender, entity)) = SOUND_EVENT_SENDER
			.get()
			.and_then(|s| Some((s, *sound.entity.get()?)))
		{
			sender
				.send((entity, SoundAction::Stop))
				.map_err(|_| eyre!("Unable to Stop Audio"))?
		}
		Ok(())
	}
}
impl Drop for Sound {
	fn drop(&mut self) {
		if let Some(sk_sound) = self.entity.take() {
			destroy_queue::add(sk_sound);
		}
		SOUND_REGISTRY.remove(self);
	}
}

impl InterfaceAspect for Interface {
	#[doc = "Create a sound node. WAV and MP3 are supported."]
	fn create_sound(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		id: u64,
		parent: Arc<Node>,
		transform: Transform,
		resource: ResourceID,
	) -> Result<()> {
		let node = Node::from_id(&calling_client, id, true);
		let parent = parent.get_aspect::<Spatial>()?;
		let transform = transform.to_mat4(true, true, true);
		let node = node.add_to_scenegraph()?;
		Spatial::add_to(&node, Some(parent.clone()), transform, false);
		Sound::add_to(&node, resource)?;
		Ok(())
	}
}

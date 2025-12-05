use super::spatial::SpatialNode;
use super::{Aspect, AspectIdentifier, Node};
use crate::bevy_int::entity_handle::EntityHandle;
use crate::core::Id;
use crate::core::client::Client;
use crate::core::error::Result;
use crate::core::registry::Registry;
use crate::core::resource::get_resource_file;
use crate::nodes::spatial::{SPATIAL_ASPECT_ALIAS_INFO, Spatial, Transform};
use bevy::audio::{PlaybackMode, Volume};
use bevy_mod_openxr::session::OxrSession;
use bevy_mod_xr::session::{XrPreDestroySession, XrSessionCreated};
use bevy_mod_xr::spaces::XrSpace;
use color_eyre::eyre::eyre;
use parking_lot::Mutex;
use stardust_xr_wire::values::ResourceID;

use bevy::prelude::*;
use bevy::transform::components::Transform as BevyTransform;
use std::sync::{Arc, OnceLock};
use std::{ffi::OsStr, path::PathBuf};

pub struct AudioNodePlugin;
impl Plugin for AudioNodePlugin {
	fn build(&self, app: &mut App) {
		app.add_systems(Update, update_sound_event);
		app.add_systems(XrSessionCreated, spawn_hmd_audio_listener);
		app.add_systems(XrPreDestroySession, despawn_hmd_audio_listener);
	}
}

fn despawn_hmd_audio_listener(mut cmds: Commands, session: Res<OxrSession>, res: Res<HmdListener>) {
	cmds.remove_resource::<HmdListener>();
	cmds.entity(res.0).despawn();
	_ = session.destroy_space(res.1);
}

fn spawn_hmd_audio_listener(mut cmds: Commands, session: Res<OxrSession>) {
	let space = session
		.create_reference_space(openxr::ReferenceSpaceType::VIEW, BevyTransform::IDENTITY)
		.unwrap();
	let listener = cmds
		.spawn((
			Name::new("HMD audio listener"),
			space.0,
			SpatialListener::new(0.2),
		))
		.id();
	cmds.insert_resource(HmdListener(listener, space.0));
}
#[derive(Resource)]
struct HmdListener(Entity, XrSpace);
fn update_sound_event(
	mut cmds: Commands,
	sinks: Query<&SpatialAudioSink>,
	asset_server: Res<AssetServer>,
) {
	for sound in SOUND_REGISTRY.get_valid_contents() {
		if sound.entity.get().is_none() {
			let handle = asset_server.load(sound.pending_audio_path.as_path());
			let entity = cmds
				.spawn((
					Name::new("Audio Node"),
					SpatialNode(Arc::downgrade(&sound.spatial)),
					AudioPlayer::new(handle),
					PlaybackSettings {
						mode: PlaybackMode::Once,
						volume: Volume::Linear(sound.volume),
						speed: 1.0,
						paused: true,
						muted: false,
						spatial: true,
						spatial_scale: None,
					},
				))
				.id();
			let entity = EntityHandle::new(entity);
			sound.spatial.set_entity(entity.clone());
			sound.entity.set(entity).unwrap();
		}
		if let Some(sink) = sound.entity.get().and_then(|e| sinks.get(e.get()).ok()) {
			if sound.play.lock().take().is_some() {
				sink.play();
			}
			if sound.stop.lock().take().is_some() {
				sink.stop();
			}
		}
	}
}

static SOUND_REGISTRY: Registry<Sound> = Registry::new();

stardust_xr_server_codegen::codegen_audio_protocol!();
pub struct Sound {
	spatial: Arc<Spatial>,

	volume: f32,
	pending_audio_path: PathBuf,
	entity: OnceLock<EntityHandle>,
	stop: Mutex<Option<()>>,
	play: Mutex<Option<()>>,
}
impl Sound {
	pub fn add_to(node: &Arc<Node>, resource_id: ResourceID) -> Result<Arc<Sound>> {
		let client = node.get_client().ok_or_else(|| eyre!("Client not found"))?;
		let pending_audio_path = get_resource_file(
			&resource_id,
			client.base_resource_prefixes.lock().iter(),
			&[OsStr::new("wav"), OsStr::new("mp3")],
		)
		.ok_or_else(|| eyre!("Resource not found"))?;
		let sound = Sound {
			spatial: node.get_aspect::<Spatial>().unwrap().clone(),
			volume: 1.0,
			pending_audio_path,
			entity: OnceLock::new(),
			stop: Mutex::new(None),
			play: Mutex::new(None),
		};
		let sound_arc = SOUND_REGISTRY.add(sound);
		node.add_aspect_raw(sound_arc.clone());
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
		sound.play.lock().replace(());
		Ok(())
	}
	fn stop(node: Arc<Node>, _calling_client: Arc<Client>) -> Result<()> {
		let sound = node.get_aspect::<Sound>().unwrap();
		sound.stop.lock().replace(());
		Ok(())
	}
}
impl Drop for Sound {
	fn drop(&mut self) {
		SOUND_REGISTRY.remove(self);
	}
}

impl InterfaceAspect for Interface {
	#[doc = "Create a sound node. WAV and MP3 are supported."]
	fn create_sound(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		id: Id,
		parent: Arc<Node>,
		transform: Transform,
		resource: ResourceID,
	) -> Result<()> {
		let node = Node::from_id(&calling_client, id, true);
		let parent = parent.get_aspect::<Spatial>()?;
		let transform = transform.to_mat4(true, true, true);
		let node = node.add_to_scenegraph()?;
		Spatial::add_to(&node, Some(parent.clone()), transform);
		Sound::add_to(&node, resource)?;
		Ok(())
	}
}

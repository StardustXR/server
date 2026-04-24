use super::spatial::SpatialNode;
use crate::bevy_int::entity_handle::EntityHandle;
use crate::core::registry::Registry;
use crate::core::resource::get_resource_file;
use crate::nodes::ProxyExt;
use crate::nodes::spatial::SpatialObject;
use crate::{PION, interface};
use bevy::audio::{PlaybackMode, Volume};
use bevy_mod_openxr::session::OxrSession;
use bevy_mod_xr::session::{XrPreDestroySession, XrSessionCreated};
use bevy_mod_xr::spaces::XrSpace;
use binderbinder::binder_object::BinderObject;
use gluon_wire::impl_transaction_handler;
use parking_lot::Mutex;

use bevy::prelude::*;
use bevy::transform::components::Transform as BevyTransform;
use stardust_xr_protocol::audio::{AudioInterfaceHandler, Sound as SoundProxy, SoundHandler};
use stardust_xr_protocol::types::Resource;
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
					SpatialNode(Arc::downgrade(&**sound.spatial)),
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

#[derive(Debug)]
pub struct Sound {
	spatial: Arc<SpatialObject>,

	volume: f32,
	pending_audio_path: PathBuf,
	entity: OnceLock<EntityHandle>,
	// Why isn't this an atomic bool or mpsc or something?
	stop: Mutex<Option<()>>,
	play: Mutex<Option<()>>,
}
impl Sound {
	pub fn new(
		spatial: Arc<SpatialObject>,
		resource_id: Resource,
		resource_prefixes: &[PathBuf],
	) -> Option<BinderObject<Sound>> {
		let pending_audio_path = get_resource_file(
			&resource_id,
			resource_prefixes,
			&[OsStr::new("wav"), OsStr::new("mp3")],
		)?;
		let sound = PION.register_object(Sound {
			spatial,
			volume: 1.0,
			pending_audio_path,
			entity: OnceLock::new(),
			stop: Mutex::new(None),
			play: Mutex::new(None),
		});
		SOUND_REGISTRY.add_raw(sound.handler_arc());
		Some(sound)
	}
}
impl SoundHandler for Sound {
	async fn play(&self, _ctx: gluon_wire::GluonCtx) {
		self.play.lock().replace(());
	}

	async fn stop(&self, _ctx: gluon_wire::GluonCtx) {
		self.stop.lock().replace(());
	}
}
impl Drop for Sound {
	fn drop(&mut self) {
		SOUND_REGISTRY.remove(self);
	}
}

interface!(AudioInterface);
impl AudioInterfaceHandler for AudioInterface {
	async fn create_sound(
		&self,
		_ctx: gluon_wire::GluonCtx,
		spatial: stardust_xr_protocol::spatial::Spatial,
		sound: Resource,
	) -> SoundProxy {
		let Some(spatial) = spatial.owned() else {
			// TODO: replace with error
			panic!("tried to create sound with invalid spatial");
		};
		let Some(sound) = Sound::new(spatial, sound, self.base_prefixes()) else {
			// TODO: replace with error
			panic!("sound resource not found");
		};
		let proxy = SoundProxy::from_handler(&sound);
		sound.to_service();
		proxy
	}
}

impl_transaction_handler!(Sound);

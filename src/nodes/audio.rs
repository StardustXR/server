use super::Node;
use crate::core::client::Client;
use crate::core::destroy_queue;
use crate::core::resource::ResourceID;
use crate::core::registry::Registry;
use crate::nodes::spatial::{Spatial, find_spatial_parent, parse_transform};
use color_eyre::eyre::{ensure, eyre, Result};
use glam::Vec4Swizzles;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use send_wrapper::SendWrapper;
use serde::Deserialize;
use stardust_xr::schemas::flex::deserialize;
use stardust_xr::values::Transform;
use std::ops::DerefMut;
use std::{sync::Arc, path::PathBuf, ffi::OsStr, fmt::Error};
use stereokit::sound::Sound as SKSound;
use stereokit::sound::SoundInstance;

static SOUND_REGISTRY: Registry<Sound> = Registry::new();

pub struct Sound {
    space: Arc<Spatial>,
    resource_id: ResourceID,
    pending_audio_path: OnceCell<PathBuf>,
    instance: Mutex<Option<SoundInstance>>,
    volume: f32,
    sk_sound: OnceCell<SendWrapper<SKSound>>,
}

impl Sound {
    pub fn add_to(node: &Arc<Node>, resource_id: ResourceID) -> Result<Arc<Sound>> {
        ensure!(
            node.spatial.get().is_some(),
            "Internal: Node does not have a spatial attached!"
        );
        let sound = Sound {
            space: node.spatial.get().unwrap().clone(),
            resource_id,
            volume: 1.0,
            instance: Mutex::new(None),
            pending_audio_path: OnceCell::new(),
            sk_sound: OnceCell::new(),
        };
        let sound_arc = SOUND_REGISTRY.add(sound);
        node.add_local_signal("play", Sound::play_flex);
        node.add_local_signal("stop", Sound::stop_flex);
        let _ = sound_arc.pending_audio_path.set(
            sound_arc
                .resource_id
                .get_file(
                    &node
                    .get_client() 
                    .ok_or_else(|| eyre!("Client not found"))?
                    .base_resource_prefixes
                    .lock()
                    .clone(),
                    &[OsStr::new("wav"), OsStr::new("mp3")]
                )
                .ok_or_else(|| eyre!("Resource not found"))?,
        );
        let _ = node.sound.set(sound_arc.clone());
        Ok(sound_arc)
    } 

    fn update(&self) {
        if let Some(instance) = self.instance.lock().deref_mut() {
            instance.set_position(self.space.global_transform().w_axis.xyz())
        }
    }

    fn play_flex(node: &Node, _calling_client: Arc<Client>, _data: &[u8]) -> Result<()> {
        let sound =node.sound.get().unwrap();
        let sk_sound = sound
            .sk_sound
            .get_or_try_init(|| -> color_eyre::eyre::Result<SendWrapper<SKSound>> {
                let pending_audio_path = sound.pending_audio_path.get().ok_or(Error)?;
                let sound = SKSound::from_file(pending_audio_path.as_path()).ok_or(Error)?;

                Ok(SendWrapper::new(sound))
            })
            .ok();
        if let Some(sk_sound) = sk_sound {
            sound.instance.lock().replace(sk_sound.play_sound(sound.space.global_transform().to_scale_rotation_translation().2, sound.volume));
        }

        Ok(())
    }

    pub fn stop_flex(node: &Node, _calling_client: Arc<Client>, _data: &[u8]) -> Result<()> {
        let sound = node.sound.get().unwrap();
        if let Some(instance) = sound.instance.lock().take() {
            instance.stop();
        }
        Ok(())
    }
}

pub fn update() {
    for sound in SOUND_REGISTRY.get_valid_contents() {
        sound.update()
    }
}

pub fn create_interface(client: &Arc<Client>) -> Result<()> {
    let node = Node::create(client, "", "audio", false);
    node.add_local_signal("create_sound", create_flex);
    node.add_to_scenegraph().map(|_| ())
}

pub fn create_flex(_node: &Node, calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
    #[derive(Deserialize)]
    struct CreateSoundInfo<'a> {
        name: &'a str,
        parent_path: &'a str,
        transform: Transform,
        resource: ResourceID,
    }
    let info: CreateSoundInfo = deserialize(data)?;
    let node = Node::create(&calling_client, "/audio/sound", info.name, true);
    let parent = find_spatial_parent(&calling_client, info.parent_path)?;
    let transform = parse_transform(info.transform, true, true, true);
    let node = node.add_to_scenegraph()?;
    Spatial::add_to(&node, Some(parent), transform, false)?;
    Sound::add_to(&node, info.resource)?;
    Ok(())
}

impl Drop for Sound {
    fn drop(&mut self) {
        if let Some(instance) = self.instance.lock().take() {
            destroy_queue::add(instance);
        }
        SOUND_REGISTRY.remove(self);
    }
    
}
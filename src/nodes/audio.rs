use super::{Aspect, Node};
use crate::core::client::Client;
use crate::core::destroy_queue;
use crate::core::registry::Registry;
use crate::core::resource::get_resource_file;
use crate::create_interface;
use crate::nodes::spatial::{Spatial, Transform};
use color_eyre::eyre::{eyre, Result};
use glam::{vec3, Vec4Swizzles};
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use stardust_xr::values::ResourceID;

use std::ops::DerefMut;
use std::sync::Arc;
use std::{ffi::OsStr, path::PathBuf};
use stereokit_rust::sound::{Sound as SkSound, SoundInst};

static SOUND_REGISTRY: Registry<Sound> = Registry::new();

stardust_xr_server_codegen::codegen_audio_protocol!();
pub struct Sound {
	space: Arc<Spatial>,

	volume: f32,
	pending_audio_path: PathBuf,
	sk_sound: OnceCell<SkSound>,
	instance: Mutex<Option<SoundInst>>,
	stop: Mutex<Option<()>>,
	play: Mutex<Option<()>>,
}
impl Sound {
	pub fn add_to(node: &Arc<Node>, resource_id: ResourceID) -> Result<Arc<Sound>> {
		let pending_audio_path = get_resource_file(
			&resource_id,
			&*node.get_client().ok_or_else(|| eyre!("Client not found"))?,
			&[OsStr::new("wav"), OsStr::new("mp3")],
		)
		.ok_or_else(|| eyre!("Resource not found"))?;
		let sound = Sound {
			space: node.get_aspect::<Spatial>().unwrap().clone(),
			volume: 1.0,
			pending_audio_path,
			sk_sound: OnceCell::new(),
			instance: Mutex::new(None),
			stop: Mutex::new(None),
			play: Mutex::new(None),
		};
		let sound_arc = SOUND_REGISTRY.add(sound);
		node.add_aspect_raw(sound_arc.clone());
		<Sound as SoundAspect>::add_node_members(node);
		Ok(sound_arc)
	}

	fn update(&self) {
		let sound = self
			.sk_sound
			.get_or_init(|| SkSound::from_file(self.pending_audio_path.clone()).unwrap());
		if self.stop.lock().take().is_some() {
			if let Some(instance) = self.instance.lock().take() {
				instance.stop();
			}
		}
		if self.instance.lock().is_none() && self.play.lock().take().is_some() {
			let instance = sound.play(vec3(0.0, 0.0, 0.0), Some(self.volume));
			self.instance.lock().replace(instance);
		}
		if let Some(instance) = self.instance.lock().deref_mut() {
			instance.position(self.space.global_transform().w_axis.xyz());
		}
	}
}
impl Aspect for Sound {
	const NAME: &'static str = "Sound";
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
		if let Some(sk_sound) = self.sk_sound.take() {
			destroy_queue::add(sk_sound);
		}
		SOUND_REGISTRY.remove(self);
	}
}

pub fn update() {
	for sound in SOUND_REGISTRY.get_valid_contents() {
		sound.update()
	}
}

create_interface!(AudioInterface);
struct AudioInterface;
impl InterfaceAspect for AudioInterface {
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

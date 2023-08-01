use super::{Message, Node};
use crate::core::client::Client;
use crate::core::destroy_queue;
use crate::core::registry::Registry;
use crate::core::resource::ResourceID;
use crate::nodes::spatial::{find_spatial_parent, parse_transform, Spatial};
use color_eyre::eyre::{ensure, eyre, Result};
use glam::{vec3, Vec4Swizzles};
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use send_wrapper::SendWrapper;
use serde::Deserialize;
use stardust_xr::schemas::flex::deserialize;
use stardust_xr::values::Transform;
use std::ops::DerefMut;
use std::{ffi::OsStr, path::PathBuf, sync::Arc};
use stereokit::{Sound as SkSound, SoundInstance, StereoKitDraw};

static SOUND_REGISTRY: Registry<Sound> = Registry::new();

pub struct Sound {
	space: Arc<Spatial>,

	volume: f32,
	pending_audio_path: PathBuf,
	sk_sound: OnceCell<SendWrapper<SkSound>>,
	instance: Mutex<Option<SoundInstance>>,
	stop: Mutex<Option<()>>,
	play: Mutex<Option<()>>,
}

impl Sound {
	pub fn add_to(node: &Arc<Node>, resource_id: ResourceID) -> Result<Arc<Sound>> {
		ensure!(
			node.spatial.get().is_some(),
			"Internal: Node does not have a spatial attached!"
		);
		let pending_audio_path = resource_id
			.get_file(
				&node
					.get_client()
					.ok_or_else(|| eyre!("Client not found"))?
					.base_resource_prefixes
					.lock()
					.clone(),
				&[OsStr::new("wav"), OsStr::new("mp3")],
			)
			.ok_or_else(|| eyre!("Resource not found"))?;
		let sound = Sound {
			space: node.spatial.get().unwrap().clone(),
			volume: 1.0,
			pending_audio_path,
			sk_sound: OnceCell::new(),
			instance: Mutex::new(None),
			stop: Mutex::new(None),
			play: Mutex::new(None),
		};
		let sound_arc = SOUND_REGISTRY.add(sound);
		node.add_local_signal("play", Sound::play_flex);
		node.add_local_signal("stop", Sound::stop_flex);
		let _ = node.sound.set(sound_arc.clone());
		Ok(sound_arc)
	}

	fn update(&self, sk: &impl StereoKitDraw) {
		let sound = self.sk_sound.get_or_init(|| {
			SendWrapper::new(sk.sound_create(self.pending_audio_path.clone()).unwrap())
		});
		if self.stop.lock().take().is_some() {
			if let Some(instance) = self.instance.lock().take() {
				sk.sound_inst_stop(instance);
			}
		}
		if self.play.lock().is_some() && self.instance.lock().is_none() {
			self.instance.lock().replace(sk.sound_play(
				sound.as_ref(),
				vec3(0.0, 0.0, 0.0),
				self.volume,
			));
		}
		if let Some(instance) = self.instance.lock().deref_mut() {
			sk.sound_inst_set_pos(*instance, self.space.global_transform().w_axis.xyz());
		}
	}

	fn play_flex(node: &Node, _calling_client: Arc<Client>, _message: Message) -> Result<()> {
		let sound = node.sound.get().unwrap();
		sound.play.lock().replace(());
		Ok(())
	}

	pub fn stop_flex(node: &Node, _calling_client: Arc<Client>, _message: Message) -> Result<()> {
		let sound = node.sound.get().unwrap();
		sound.stop.lock().replace(());
		Ok(())
	}
}

pub fn update(sk: &impl StereoKitDraw) {
	for sound in SOUND_REGISTRY.get_valid_contents() {
		sound.update(sk)
	}
}

pub fn create_interface(client: &Arc<Client>) -> Result<()> {
	let node = Node::create(client, "", "audio", false);
	node.add_local_signal("create_sound", create_flex);
	node.add_to_scenegraph().map(|_| ())
}

pub fn create_flex(_node: &Node, calling_client: Arc<Client>, message: Message) -> Result<()> {
	#[derive(Deserialize)]
	struct CreateSoundInfo<'a> {
		name: &'a str,
		parent_path: &'a str,
		transform: Transform,
		resource: ResourceID,
	}
	let info: CreateSoundInfo = deserialize(message.as_ref())?;
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

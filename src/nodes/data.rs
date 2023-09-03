use super::alias::AliasInfo;
use super::fields::Field;
use super::spatial::{parse_transform, Spatial};
use super::{Alias, Message, Node};
use crate::core::client::Client;
use crate::core::node_collections::LifeLinkedNodeMap;
use crate::core::registry::Registry;
use crate::core::scenegraph::MethodResponseSender;
use crate::nodes::fields::{find_field, FIELD_ALIAS_INFO};
use crate::nodes::spatial::find_spatial_parent;
use color_eyre::eyre::{bail, ensure, eyre, Result};
use glam::vec3a;
use lazy_static::lazy_static;
use mint::{Quaternion, Vector3};
use nanoid::nanoid;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use stardust_xr::schemas::flex::{deserialize, flexbuffers, serialize};
use stardust_xr::values::Transform;
use std::sync::{Arc, Weak};

lazy_static! {
	pub static ref KEYMAPS: Mutex<FxHashMap<String, String>> = Mutex::new(FxHashMap::default());
}

static PULSE_SENDER_REGISTRY: Registry<PulseSender> = Registry::new();
pub static PULSE_RECEIVER_REGISTRY: Registry<PulseReceiver> = Registry::new();

pub fn mask_matches(mask_map_lesser: &Mask, mask_map_greater: &Mask) -> bool {
	(|| -> Result<_> {
		for key in mask_map_lesser.get_mask()?.iter_keys() {
			let lesser_key = mask_map_lesser.get_mask()?.index(key)?;
			let greater_key = mask_map_greater.get_mask()?.index(key)?;
			if !lesser_key.flexbuffer_type().is_null()
				&& lesser_key.flexbuffer_type() != greater_key.flexbuffer_type()
			{
				return Err(flexbuffers::ReaderError::InvalidPackedType {}.into());
			}
		}
		Ok(())
	})()
	.is_ok()
}

pub struct Mask(pub Vec<u8>);
impl Mask {
	pub fn from_struct<T: Default + Serialize>() -> Self {
		let mut serializer = flexbuffers::FlexbufferSerializer::new();
		T::default().serialize(&mut serializer).unwrap();
		Mask(serializer.take_buffer())
	}
	pub fn get_mask(&self) -> Result<flexbuffers::MapReader<&[u8]>> {
		flexbuffers::Reader::get_root(self.0.as_slice())
			.map_err(|_| eyre!("Mask is not a valid flexbuffer"))?
			.get_map()
			.map_err(|_| eyre!("Mask is not a valid map"))
	}
}

#[derive(Serialize, Deserialize)]
struct SendDataInfo<'a> {
	uid: &'a str,
	data: Vec<u8>,
}

pub struct PulseSender {
	uid: String,
	node: Weak<Node>,
	pub mask: Mask,
	aliases: LifeLinkedNodeMap<String>,
}
impl PulseSender {
	pub fn add_to(node: &Arc<Node>, mask: Mask) -> Result<Arc<PulseSender>> {
		ensure!(
			node.spatial.get().is_some(),
			"Internal: Node does not have a spatial attached!"
		);

		let sender = PulseSender {
			uid: nanoid!(),
			node: Arc::downgrade(node),
			mask,
			aliases: LifeLinkedNodeMap::default(),
		};
		let sender = PULSE_SENDER_REGISTRY.add(sender);
		let _ = node.pulse_sender.set(sender.clone());
		node.add_local_signal("send_data", PulseSender::send_data_flex);
		for receiver in PULSE_RECEIVER_REGISTRY.get_valid_contents() {
			sender.handle_new_receiver(&receiver);
		}
		Ok(sender.clone())
	}
	fn handle_new_receiver(&self, receiver: &PulseReceiver) {
		if !mask_matches(&self.mask, &receiver.mask) {
			return;
		}
		let Some(tx_node) = self.node.upgrade() else {return};
		let Some(tx_client) = tx_node.get_client() else {return};
		let Some(rx_node) = receiver.node.upgrade() else {return};
		// Receiver itself
		let rx_alias = Alias::create(
			&tx_client,
			tx_node.get_path(),
			receiver.uid.as_str(),
			&rx_node,
			AliasInfo {
				server_methods: vec!["sendData", "getTransform"],
				..Default::default()
			},
		);
		if let Ok(rx_alias) = rx_alias {
			self.aliases.add(receiver.uid.clone(), &rx_alias);

			if let Some(rx_field_node) = receiver.field.spatial_ref().node.upgrade() {
				// Receiver's field
				let rx_field_alias = Alias::create(
					&tx_client,
					rx_alias.get_path(),
					"field",
					&rx_field_node,
					FIELD_ALIAS_INFO.clone(),
				);
				if let Ok(rx_field_alias) = rx_field_alias {
					self.aliases
						.add(receiver.uid.clone() + "-field", &rx_field_alias);
				}
			}
		}

		#[derive(Serialize)]
		struct NewReceiverInfo<'a> {
			uid: &'a str,
			distance: f32,
			position: Vector3<f32>,
			rotation: Quaternion<f32>,
		}

		let (_, rotation, position) = Spatial::space_to_space_matrix(
			rx_node.spatial.get().map(|s| s.as_ref()),
			tx_node.spatial.get().map(|s| s.as_ref()),
		)
		.to_scale_rotation_translation();

		let info = NewReceiverInfo {
			uid: &receiver.uid,
			distance: receiver
				.field
				.distance(tx_node.spatial.get().unwrap(), vec3a(0.0, 0.0, 0.0)),
			position: position.into(),
			rotation: rotation.into(),
		};

		let Ok(data) = serialize(info) else {return};
		let _ = tx_node.send_remote_signal("new_receiver", data);
	}

	fn handle_drop_receiver(&self, receiver: &PulseReceiver) {
		let uid = receiver.uid.as_str();
		self.aliases.remove(uid);
		self.aliases.remove(&(uid.to_string() + "-field"));
		let Some(tx_node) = self.node.upgrade() else {return};
		let Ok(data) = serialize(&uid) else {return};
		let _ = tx_node.send_remote_signal("drop_receiver", data);
	}

	fn send_data_flex(node: &Node, calling_client: Arc<Client>, message: Message) -> Result<()> {
		let info: SendDataInfo = deserialize(message.as_ref())?;
		let receiver_node = calling_client.get_node("Pulse receiver", info.uid)?;
		let receiver =
			receiver_node.get_aspect("Pulse Receiver", "pulse receiver", |n| &n.pulse_receiver)?;
		let receiver_mask = &receiver_node
			.get_aspect("Pulse receiver", "pulse receiver", |node| {
				&node.pulse_receiver
			})?
			.mask;

		let data_mask = Mask(info.data);
		data_mask.get_mask()?;
		ensure!(
			mask_matches(receiver_mask, &data_mask),
			"Message does not contain the same keys as the receiver's mask"
		);
		receiver.send_data(&node.pulse_sender.get().unwrap().uid, data_mask.0)
	}
}
impl Drop for PulseSender {
	fn drop(&mut self) {
		PULSE_SENDER_REGISTRY.remove(self);
	}
}

pub struct PulseReceiver {
	uid: String,
	pub node: Weak<Node>,
	pub field: Arc<Field>,
	pub mask: Mask,
}
impl PulseReceiver {
	pub fn add_to(node: &Arc<Node>, field: Arc<Field>, mask: Mask) -> Result<Arc<PulseReceiver>> {
		ensure!(
			node.spatial.get().is_some(),
			"Internal: Node does not have a spatial attached!"
		);

		let receiver = PulseReceiver {
			uid: nanoid!(),
			node: Arc::downgrade(node),
			field,
			mask,
		};
		let receiver = PULSE_RECEIVER_REGISTRY.add(receiver);

		for sender in PULSE_SENDER_REGISTRY.get_valid_contents() {
			sender.handle_new_receiver(&receiver);
		}
		let _ = node.pulse_receiver.set(receiver.clone());
		Ok(receiver)
	}

	pub fn send_data(&self, uid: &str, data: Vec<u8>) -> Result<()> {
		if let Some(node) = self.node.upgrade() {
			node.send_remote_signal("data", serialize(SendDataInfo { uid, data })?)?;
		}
		Ok(())
	}
}

impl Drop for PulseReceiver {
	fn drop(&mut self) {
		PULSE_RECEIVER_REGISTRY.remove(self);
		for sender in PULSE_SENDER_REGISTRY.get_valid_contents() {
			sender.handle_drop_receiver(self);
		}
	}
}

pub fn create_interface(client: &Arc<Client>) -> Result<()> {
	let node = Node::create(client, "", "data", false);
	node.add_local_signal("create_pulse_sender", create_pulse_sender_flex);
	node.add_local_signal("create_pulse_receiver", create_pulse_receiver_flex);
	node.add_local_method("register_keymap", register_keymap_flex);
	node.add_local_method("get_keymap", get_keymap_flex);
	node.add_to_scenegraph().map(|_| ())
}

pub fn create_pulse_sender_flex(
	_node: &Node,
	calling_client: Arc<Client>,
	message: Message,
) -> Result<()> {
	#[derive(Deserialize)]
	struct CreatePulseSenderInfo<'a> {
		name: &'a str,
		parent_path: &'a str,
		transform: Transform,
		mask: Vec<u8>,
	}
	let info: CreatePulseSenderInfo = deserialize(message.as_ref())?;
	let node = Node::create(&calling_client, "/data/sender", info.name, true);
	let parent = find_spatial_parent(&calling_client, info.parent_path)?;
	let transform = parse_transform(info.transform, true, true, false);

	let mask = Mask(info.mask);
	mask.get_mask()?;

	let node = node.add_to_scenegraph()?;
	Spatial::add_to(&node, Some(parent), transform, false)?;
	PulseSender::add_to(&node, mask)?;
	Ok(())
}

pub fn create_pulse_receiver_flex(
	_node: &Node,
	calling_client: Arc<Client>,
	message: Message,
) -> Result<()> {
	#[derive(Deserialize)]
	struct CreatePulseReceiverInfo<'a> {
		name: &'a str,
		parent_path: &'a str,
		transform: Transform,
		field_path: &'a str,
		mask: Vec<u8>,
	}
	let info: CreatePulseReceiverInfo = deserialize(message.as_ref())?;
	let node = Node::create(&calling_client, "/data/receiver", info.name, true);
	let parent = find_spatial_parent(&calling_client, info.parent_path)?;
	let transform = parse_transform(info.transform, true, true, false);
	let field = find_field(&calling_client, info.field_path)?;
	let mask = Mask(info.mask);
	mask.get_mask()?;

	let node = node.add_to_scenegraph()?;
	Spatial::add_to(&node, Some(parent), transform, false)?;
	PulseReceiver::add_to(&node, field, mask)?;
	Ok(())
}

pub fn register_keymap_flex(
	_node: &Node,
	_calling_client: Arc<Client>,
	message: Message,
	response: MethodResponseSender,
) {
	response.wrap_sync(move || {
		let keymap: String = deserialize(message.as_ref())?;
		let mut keymaps = KEYMAPS.lock();
		if let Some(found_keymap_id) = keymaps
			.iter()
			.filter(|(_k, v)| *v == &keymap)
			.map(|(k, _v)| k)
			.last()
		{
			return Ok(serialize(found_keymap_id)?.into());
		}

		let generated_id = nanoid!();
		keymaps.insert(generated_id.clone(), keymap);

		Ok(serialize(generated_id)?.into())
	});
}
pub fn get_keymap_flex(
	_node: &Node,
	_calling_client: Arc<Client>,
	message: Message,
	response: MethodResponseSender,
) {
	response.wrap_sync(move || {
		let keymap_id: &str = deserialize(message.as_ref())?;
		let keymaps = KEYMAPS.lock();
		let Some(keymap) = keymaps.get(keymap_id) else {bail!("Could not find keymap. Try registering it")};

		Ok(serialize(keymap)?.into())
	});
}

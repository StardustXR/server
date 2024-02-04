use super::alias::AliasInfo;
use super::fields::get_field;
use super::spatial::{get_spatial, parse_transform, Spatial};
use super::{Alias, Node};
use crate::core::client::Client;
use crate::core::node_collections::LifeLinkedNodeMap;
use crate::core::registry::Registry;
use crate::create_interface;
use crate::nodes::fields::FIELD_ALIAS_INFO;
use crate::nodes::spatial::Transform;
use color_eyre::eyre::{bail, ensure, eyre, Result};
use lazy_static::lazy_static;
use nanoid::nanoid;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use stardust_xr::schemas::flex::flexbuffers;
use stardust_xr::values::Datamap;
use std::sync::{Arc, Weak};

lazy_static! {
	pub static ref KEYMAPS: Mutex<FxHashMap<String, String>> = Mutex::new(FxHashMap::default());
}

static PULSE_SENDER_REGISTRY: Registry<PulseSender> = Registry::new();
pub static PULSE_RECEIVER_REGISTRY: Registry<PulseReceiver> = Registry::new();

pub fn get_mask(datamap: &Datamap) -> Result<flexbuffers::MapReader<&[u8]>> {
	flexbuffers::Reader::get_root(datamap.raw().as_slice())
		.map_err(|_| eyre!("Mask is not a valid flexbuffer"))?
		.get_map()
		.map_err(|_| eyre!("Mask is not a valid map"))
}
pub fn mask_matches(mask_map_lesser: &Datamap, mask_map_greater: &Datamap) -> bool {
	(|| -> Result<_> {
		for key in get_mask(mask_map_lesser)?.iter_keys() {
			let lesser_key = get_mask(mask_map_lesser)?.index(key)?;
			let greater_key = get_mask(mask_map_greater)?.index(key)?;
			// otherwise zero-length vectors don't count the same as a single type vector
			if lesser_key.flexbuffer_type().is_heterogenous_vector()
				&& lesser_key.as_vector().len() == 0
				&& greater_key.flexbuffer_type().is_vector()
			{
				continue;
			}
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

stardust_xr_server_codegen::codegen_data_protocol!();

pub struct PulseSender {
	node: Weak<Node>,
	pub mask: Datamap,
	aliases: LifeLinkedNodeMap<String>,
}
impl PulseSender {
	pub fn add_to(node: &Arc<Node>, mask: Datamap) -> Result<Arc<PulseSender>> {
		ensure!(
			node.spatial.get().is_some(),
			"Internal: Node does not have a spatial attached!"
		);

		let sender = PulseSender {
			node: Arc::downgrade(node),
			mask,
			aliases: LifeLinkedNodeMap::default(),
		};
		let sender = PULSE_SENDER_REGISTRY.add(sender);
		let _ = node.pulse_sender.set(sender.clone());
		for receiver in PULSE_RECEIVER_REGISTRY.get_valid_contents() {
			sender.handle_new_receiver(&receiver);
		}
		Ok(sender.clone())
	}
	fn handle_new_receiver(&self, receiver: &PulseReceiver) {
		if !mask_matches(&self.mask, &receiver.mask) {
			return;
		}
		let Some(tx_node) = self.node.upgrade() else {
			return;
		};
		let Some(tx_client) = tx_node.get_client() else {
			return;
		};
		let Some(rx_node) = receiver.node.upgrade() else {
			return;
		};
		// Receiver itself
		let rx_alias = Alias::create(
			&tx_client,
			tx_node.get_path(),
			receiver.uid.as_str(),
			&rx_node,
			AliasInfo {
				server_methods: vec!["send_data"],
				..Default::default()
			},
		);
		let Ok(rx_alias) = rx_alias else { return };
		self.aliases.add(receiver.uid.clone(), &rx_alias);

		// Receiver's field
		let Ok(rx_field_alias) = Alias::create(
			&tx_client,
			rx_alias.get_path(),
			"field",
			&rx_node.pulse_receiver.get().unwrap().field_node,
			FIELD_ALIAS_INFO.clone(),
		) else {
			return;
		};
		self.aliases
			.add(receiver.uid.clone() + "-field", &rx_field_alias);

		let _ =
			pulse_sender_client::new_receiver(&tx_node, &receiver.uid, &rx_alias, &rx_field_alias);
	}

	fn handle_drop_receiver(&self, receiver: &PulseReceiver) {
		let uid = receiver.uid.as_str();
		self.aliases.remove(uid);
		self.aliases.remove(&(uid.to_string() + "-field"));
		let Some(tx_node) = self.node.upgrade() else {
			return;
		};
		let _ = pulse_sender_client::drop_receiver(&tx_node, uid);
	}
}
impl PulseSenderAspect for PulseSender {}
impl Drop for PulseSender {
	fn drop(&mut self) {
		PULSE_SENDER_REGISTRY.remove(self);
	}
}

pub struct PulseReceiver {
	uid: String,
	pub node: Weak<Node>,
	pub field_node: Arc<Node>,
	pub mask: Datamap,
}
impl PulseReceiver {
	pub fn add_to(
		node: &Arc<Node>,
		field_node: Arc<Node>,
		mask: Datamap,
	) -> Result<Arc<PulseReceiver>> {
		ensure!(
			node.spatial.get().is_some(),
			"Internal: Node does not have a spatial attached!"
		);

		let receiver = PulseReceiver {
			uid: nanoid!(),
			node: Arc::downgrade(node),
			field_node,
			mask,
		};
		let receiver = PULSE_RECEIVER_REGISTRY.add(receiver);

		for sender in PULSE_SENDER_REGISTRY.get_valid_contents() {
			sender.handle_new_receiver(&receiver);
		}
		let _ = node.pulse_receiver.set(receiver.clone());
		Ok(receiver)
	}
}
impl PulseReceiverAspect for PulseReceiver {
	fn send_data(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		sender: Arc<Node>,
		data: Datamap,
	) -> Result<()> {
		let this_receiver = node.pulse_receiver.get().unwrap();

		ensure!(
			mask_matches(&this_receiver.mask, &data),
			"Message ({data:?}) does not contain the same keys as the receiver's mask ({:?})",
			this_receiver.mask
		);
		pulse_receiver_client::data(&node, &sender.uid, &data)?;
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

create_interface!(DataInterface, DataInterfaceAspect, "/data");
struct DataInterface;
impl DataInterfaceAspect for DataInterface {
	fn create_pulse_sender(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		name: String,
		parent: Arc<Node>,
		transform: Transform,
		mask: Datamap,
	) -> Result<()> {
		get_mask(&mask)?;
		let node = Node::create_parent_name(
			&calling_client,
			Self::CREATE_PULSE_SENDER_PARENT_PATH,
			&name,
			true,
		);
		let parent = get_spatial(&parent, "Spatial parent")?;
		let transform = transform.to_mat4(true, true, false);

		let node = node.add_to_scenegraph()?;
		Spatial::add_to(&node, Some(parent), transform, false)?;
		PulseSender::add_to(&node, mask)?;
		Ok(())
	}

	fn create_pulse_receiver(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		name: String,
		parent: Arc<Node>,
		transform: Transform,
		field: Arc<Node>,
		mask: Datamap,
	) -> Result<()> {
		get_mask(&mask)?;
		let node = Node::create_parent_name(
			&calling_client,
			Self::CREATE_PULSE_RECEIVER_PARENT_PATH,
			&name,
			true,
		);
		let parent = get_spatial(&parent, "Spatial parent")?;
		let transform = parse_transform(transform, true, true, false);
		get_field(&field)?;

		let node = node.add_to_scenegraph()?;
		Spatial::add_to(&node, Some(parent), transform, false)?;
		PulseReceiver::add_to(&node, field, mask)?;
		Ok(())
	}

	async fn register_keymap(
		_node: Arc<Node>,
		_calling_client: Arc<Client>,
		keymap: String,
	) -> Result<String> {
		let mut keymaps = KEYMAPS.lock();
		if let Some(found_keymap_id) = keymaps
			.iter()
			.filter(|(_k, v)| *v == &keymap)
			.map(|(k, _v)| k)
			.last()
		{
			return Ok(found_keymap_id.clone());
		}

		let generated_id = nanoid!();
		keymaps.insert(generated_id.clone(), keymap);

		Ok(generated_id)
	}

	async fn get_keymap(
		_node: Arc<Node>,
		_calling_client: Arc<Client>,
		keymap_id: String,
	) -> Result<String> {
		let keymaps = KEYMAPS.lock();
		let Some(keymap) = keymaps.get(&keymap_id) else {
			bail!("Could not find keymap. Try registering it")
		};

		Ok(keymap.clone())
	}
}

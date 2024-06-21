use super::alias::AliasList;
use super::fields::Field;
use super::spatial::{parse_transform, Spatial};
use super::{Alias, Aspect, Node};
use crate::core::client::Client;
use crate::core::registry::Registry;
use crate::create_interface;
use crate::nodes::fields::FIELD_ALIAS_INFO;
use crate::nodes::spatial::Transform;
use crate::nodes::spatial::SPATIAL_ASPECT_ALIAS_INFO;
use color_eyre::eyre::{bail, ensure, eyre, Result};
use lazy_static::lazy_static;
use parking_lot::Mutex;
use slotmap::{DefaultKey, Key, KeyData, SlotMap};
use stardust_xr::schemas::flex::flexbuffers;
use stardust_xr::values::Datamap;
use std::sync::{Arc, Weak};

lazy_static! {
	pub static ref KEYMAPS: Mutex<SlotMap<DefaultKey, String>> = Mutex::new(SlotMap::default());
}

// TODO: probably just use d-bus for this stuff (custom protocol for exporting spatials as refs) because the mask stuff is just too confusing

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
				&& lesser_key.as_vector().is_empty()
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
	aliases: AliasList,
	field_aliases: AliasList,
}
impl PulseSender {
	pub fn add_to(node: &Arc<Node>, mask: Datamap) -> Result<Arc<PulseSender>> {
		let sender = PulseSender {
			node: Arc::downgrade(node),
			mask,
			aliases: AliasList::default(),
			field_aliases: AliasList::default(),
		};

		// <PulseSender as PulseSenderAspect>::add_node_members(node);
		let sender = PULSE_SENDER_REGISTRY.add(sender);
		node.add_aspect_raw(sender.clone());
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
		let Ok(rx_alias) = Alias::create(
			&rx_node,
			&tx_client,
			PULSE_RECEIVER_ASPECT_ALIAS_INFO.clone(),
			Some(&self.aliases),
		) else {
			return;
		};

		// Receiver's field
		let Ok(rx_field_alias) = Alias::create(
			&rx_node
				.get_aspect::<PulseReceiver>()
				.unwrap()
				.field
				.spatial
				.node()
				.unwrap(),
			&tx_client,
			FIELD_ALIAS_INFO.clone(),
			Some(&self.aliases),
		) else {
			return;
		};

		let _ = pulse_sender_client::new_receiver(&tx_node, &rx_alias, &rx_field_alias);
	}

	fn handle_drop_receiver(&self, receiver: &PulseReceiver) {
		let Some(node) = receiver.node.upgrade() else {
			return;
		};
		self.aliases.remove_aspect(receiver);
		self.field_aliases.remove_aspect(receiver.field.as_ref());
		let Some(tx_node) = self.node.upgrade() else {
			return;
		};
		let _ = pulse_sender_client::drop_receiver(&tx_node, node.get_id());
	}
}
impl Aspect for PulseSender {
	const NAME: &'static str = "PulseSender";
}
impl PulseSenderAspect for PulseSender {}
impl Drop for PulseSender {
	fn drop(&mut self) {
		PULSE_SENDER_REGISTRY.remove(self);
	}
}

pub struct PulseReceiver {
	pub node: Weak<Node>,
	pub field: Arc<Field>,
	pub mask: Datamap,
}
impl PulseReceiver {
	pub fn add_to(
		node: &Arc<Node>,
		field: Arc<Field>,
		mask: Datamap,
	) -> Result<Arc<PulseReceiver>> {
		let receiver = PulseReceiver {
			node: Arc::downgrade(node),
			field,
			mask,
		};
		let receiver = PULSE_RECEIVER_REGISTRY.add(receiver);

		<PulseReceiver as PulseReceiverAspect>::add_node_members(node);
		node.add_aspect_raw(receiver.clone());
		for sender in PULSE_SENDER_REGISTRY.get_valid_contents() {
			sender.handle_new_receiver(&receiver);
		}
		Ok(receiver)
	}
}
impl Aspect for PulseReceiver {
	const NAME: &'static str = "PulseReceiver";
}
impl PulseReceiverAspect for PulseReceiver {
	fn send_data(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		sender: Arc<Node>,
		data: Datamap,
	) -> Result<()> {
		let this_receiver = node.get_aspect::<PulseReceiver>().unwrap();

		ensure!(
			mask_matches(&this_receiver.mask, &data),
			"Message ({data:?}) does not contain the same keys as the receiver's mask ({:?})",
			this_receiver.mask
		);
		pulse_receiver_client::data(&node, &sender, &data)?;
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

create_interface!(DataInterface);
struct DataInterface;
impl InterfaceAspect for DataInterface {
	fn create_pulse_sender(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		id: u64,
		parent: Arc<Node>,
		transform: Transform,
		mask: Datamap,
	) -> Result<()> {
		get_mask(&mask)?;
		let node = Node::from_id(&calling_client, id, true);
		let parent = parent.get_aspect::<Spatial>()?;
		let transform = transform.to_mat4(true, true, false);

		let node = node.add_to_scenegraph()?;
		Spatial::add_to(&node, Some(parent.clone()), transform, false);
		PulseSender::add_to(&node, mask)?;
		Ok(())
	}

	fn create_pulse_receiver(
		_node: Arc<Node>,
		calling_client: Arc<Client>,
		id: u64,
		parent: Arc<Node>,
		transform: Transform,
		field: Arc<Node>,
		mask: Datamap,
	) -> Result<()> {
		get_mask(&mask)?;
		let node = Node::from_id(&calling_client, id, true);
		let parent = parent.get_aspect::<Spatial>()?;
		let transform = parse_transform(transform, true, true, false);
		let field = field.get_aspect::<Field>()?;

		let node = node.add_to_scenegraph()?;
		Spatial::add_to(&node, Some(parent.clone()), transform, false);
		PulseReceiver::add_to(&node, field, mask)?;
		Ok(())
	}

	async fn register_keymap(
		_node: Arc<Node>,
		_calling_client: Arc<Client>,
		keymap: String,
	) -> Result<u64> {
		let mut keymaps = KEYMAPS.lock();
		if let Some(found_keymap_id) = keymaps
			.iter()
			.filter(|(_k, v)| *v == &keymap)
			.map(|(k, _v)| k)
			.last()
		{
			return Ok(found_keymap_id.data().as_ffi());
		}

		let key = keymaps.insert(keymap);
		Ok(key.data().as_ffi())
	}

	async fn get_keymap(
		_node: Arc<Node>,
		_calling_client: Arc<Client>,
		keymap_id: u64,
	) -> Result<String> {
		let keymaps = KEYMAPS.lock();
		let Some(keymap) = keymaps.get(KeyData::from_ffi(keymap_id).into()) else {
			bail!("Could not find keymap. Try registering it")
		};

		Ok(keymap.clone())
	}
}

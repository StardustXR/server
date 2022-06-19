use super::core::{Alias, Node};
use super::field::Field;
use super::spatial::{get_spatial_parent_flex, get_transform_pose_flex, Spatial};
use crate::core::client::Client;
use crate::core::nodelist::LifeLinkedNodeList;
use crate::core::registry::Registry;
use anyhow::{anyhow, ensure, Result};
use glam::{vec3a, Mat4};
use lazy_static::lazy_static;
use libstardustxr::flex::flexbuffer_from_vector_arguments;
use libstardustxr::{flex_to_quat, flex_to_vec3};
use parking_lot::{Mutex, RwLock};
use std::sync::{Arc, Weak};

lazy_static! {
	static ref PULSE_SENDER_REGISTRY: Registry<PulseSender> = Default::default();
	static ref PULSE_RECEIVER_REGISTRY: Registry<PulseReceiver> = Default::default();
}

fn mask_matches(mask_map_lesser: &Mask, mask_map_greater: &Mask) -> bool {
	(|| -> Result<_> {
		for key in mask_map_lesser.get_mask()?.iter_keys() {
			let lesser_key_type = mask_map_lesser.get_mask()?.index(key)?.flexbuffer_type();
			let greater_key_type = mask_map_greater.get_mask()?.index(key)?.flexbuffer_type();
			if lesser_key_type != greater_key_type {
				return Err(flexbuffers::ReaderError::InvalidPackedType {}.into());
			}
		}
		Ok(())
	})()
	.is_ok()
}

type MaskMapGetFn = fn(&[u8]) -> Result<flexbuffers::MapReader<&[u8]>>;
pub struct Mask {
	binary: Vec<u8>,
	get_fn: MaskMapGetFn,
}
impl Mask {
	pub fn get_mask(&self) -> Result<flexbuffers::MapReader<&[u8]>> {
		(self.get_fn)(self.binary.as_slice())
	}
	pub fn set_mask(&mut self, binary: Vec<u8>, get_fn: MaskMapGetFn) {
		self.binary = binary;
		self.get_fn = get_fn;
	}
}
impl Default for Mask {
	fn default() -> Self {
		Mask {
			binary: Default::default(),
			get_fn: mask_get_err,
		}
	}
}
fn mask_get_err(_binary: &[u8]) -> Result<flexbuffers::MapReader<&[u8]>> {
	Err(anyhow!("You need to call setMask to set the mask!"))
}
fn mask_get_map_at_root(binary: &[u8]) -> Result<flexbuffers::MapReader<&[u8]>> {
	flexbuffers::Reader::get_root(binary)
		.map_err(|_| anyhow!("Mask is not a valid flexbuffer"))?
		.get_map()
		.map_err(|_| anyhow!("Mask is not a valid map"))
}

#[derive(Default)]
pub struct PulseSender {
	mask: RwLock<Mask>,
	aliases: LifeLinkedNodeList,
}
impl PulseSender {
	pub fn add_to(node: &Arc<Node>) -> Result<()> {
		ensure!(
			node.spatial.get().is_some(),
			"Internal: Node does not have a spatial attached!"
		);

		let sender = Default::default();
		let sender = PULSE_SENDER_REGISTRY.add(sender)?;
		let _ = node.pulse_sender.set(sender);
		node.add_local_signal("setMask", PulseSender::set_mask_flex);
		node.add_local_method("getReceivers", PulseSender::get_receivers_flex);
		Ok(())
	}
	pub fn set_mask_flex(node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		ensure!(
			node.pulse_sender.get().is_some(),
			"Internal: Node does not have a pulse sender aspect"
		);
		node.pulse_sender
			.get()
			.unwrap()
			.mask
			.write()
			.set_mask(data.to_vec(), mask_get_map_at_root);
		Ok(())
	}
	fn get_receivers_flex(
		node: &Node,
		calling_client: Arc<Client>,
		_data: &[u8],
	) -> Result<Vec<u8>> {
		let sender_spatial = node
			.spatial
			.get()
			.ok_or_else(|| anyhow!("Node does not have a spatial aspect!"))?;
		let sender = node
			.pulse_sender
			.get()
			.ok_or_else(|| anyhow!("Node does not have a sender aspect!"))?;
		let valid_receivers = PULSE_RECEIVER_REGISTRY.get_valid_contents();
		let mut distance_sorted_receivers: Vec<(f32, &PulseReceiver)> = valid_receivers
			.iter()
			.filter(|receiver| receiver.get_field().is_some())
			.filter(|receiver| mask_matches(&*sender.mask.read(), &*receiver.mask.read()))
			.map(|receiver| {
				(
					receiver
						.get_field()
						.unwrap()
						.distance(sender_spatial, vec3a(0_f32, 0_f32, 0_f32)),
					receiver.as_ref(),
				)
			})
			.collect();
		distance_sorted_receivers.sort_by(|(d1, _), (d2, _)| d1.partial_cmp(d2).unwrap());

		Ok(flexbuffer_from_vector_arguments(move |fbb| {
			sender.aliases.clear();
			for (i, (_, receiver)) in distance_sorted_receivers.iter().enumerate() {
				let receiver_alias = Node::create(node.get_path(), receiver.uid.as_str(), false);
				let receiver_alias = calling_client.scenegraph.add_node(receiver_alias);
				Alias::add_to(
					&receiver_alias,
					receiver.node.upgrade().as_ref().unwrap(),
					vec![],
					vec!["sendData"],
				);
				sender.aliases.add(Arc::downgrade(&receiver_alias));
				fbb.push(receiver.uid.as_str());
			}
		}))
	}
}
impl Drop for PulseSender {
	fn drop(&mut self) {
		let _ = PULSE_SENDER_REGISTRY.remove(self);
	}
}

pub struct PulseReceiver {
	uid: String,
	node: Weak<Node>,
	pub mask: RwLock<Mask>,
	field: Weak<Field>,
}
impl<'a> PulseReceiver {
	pub fn add_to(node: &Arc<Node>, field: Arc<Field>) -> Result<()> {
		ensure!(
			node.spatial.get().is_some(),
			"Internal: Node does not have a spatial attached!"
		);

		let receiver = PulseReceiver {
			uid: node.uid.clone(),
			node: Arc::downgrade(node),
			field: Arc::downgrade(&field),
			mask: Default::default(),
		};
		let receiver = PULSE_RECEIVER_REGISTRY.add(receiver)?;
		let _ = node.pulse_receiver.set(receiver);
		node.add_local_signal("setMask", PulseReceiver::set_mask_flex);
		node.add_local_signal("sendData", PulseReceiver::send_data_flex);
		Ok(())
	}
	fn get_field(&self) -> Option<Arc<Field>> {
		self.field.upgrade()
	}
	fn send_data_flex(node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		ensure!(
			node.pulse_receiver.get().is_some(),
			"Internal: Node does not have a pulse receiver aspect"
		);
		let receiver_mask = node.pulse_receiver.get().unwrap().mask.read();
		let data_mask = Mask {
			binary: data.to_vec(),
			get_fn: mask_get_map_at_root,
		};
		if !mask_matches(&receiver_mask, &data_mask) {
			return Err(anyhow!(
				"Message does not contain the same keys as the receiver mask"
			));
		}
		drop(receiver_mask);
		node.send_remote_signal("pulse", data)?;
		Ok(())
	}
	fn set_mask_flex(node: &Node, _calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		ensure!(
			node.pulse_receiver.get().is_some(),
			"Internal: Node does not have a pulse receiver aspect"
		);
		node.pulse_receiver
			.get()
			.unwrap()
			.mask
			.write()
			.set_mask(data.to_vec(), mask_get_map_at_root);
		Ok(())
	}
}

impl Drop for PulseReceiver {
	fn drop(&mut self) {
		let _ = PULSE_RECEIVER_REGISTRY.remove(self);
	}
}

pub fn create_interface(client: &Arc<Client>) {
	let node = Node::create("", "data", false);
	node.add_local_signal("createPulseSender", create_pulse_sender_flex);
	node.add_local_signal("createPulseReceiver", create_pulse_receiver_flex);
	client.scenegraph.add_node(node);
}

// pub fn mask_get_map_pulse_sender_create_args(mask: &Mask) -> Result<flexbuffers::MapReader<&[u8]>> {
// 	flexbuffers::Reader::get_root(mask.binary.as_slice())
// 		.map_err(|_| anyhow!("Mask is not a valid flexbuffer"))?
// 		.get_vector()?
// 		.index(4)?
// 		.get_map()
// 		.map_err(|_| anyhow!("Mask is not a valid map"))
// }
pub fn create_pulse_sender_flex(
	_node: &Node,
	calling_client: Arc<Client>,
	data: &[u8],
) -> Result<()> {
	let root = flexbuffers::Reader::get_root(data)?;
	let flex_vec = root.get_vector()?;
	let node = Node::create("/data/sender", flex_vec.idx(0).get_str()?, true);
	let parent = get_spatial_parent_flex(&calling_client, flex_vec.idx(1).get_str()?)?;
	let transform = Mat4::from_rotation_translation(
		flex_to_quat!(flex_vec.idx(3))
			.ok_or_else(|| anyhow!("Rotation not found"))?
			.into(),
		flex_to_vec3!(flex_vec.idx(2))
			.ok_or_else(|| anyhow!("Position not found"))?
			.into(),
	);
	let node_rc = calling_client.scenegraph.add_node(node);
	Spatial::add_to(&node_rc, Some(parent), transform)?;
	PulseSender::add_to(&node_rc)?;
	Ok(())
}

// pub fn mask_get_map_pulse_receiver_create_args(
// 	mask: &Mask,
// ) -> Result<flexbuffers::MapReader<&[u8]>> {
// 	flexbuffers::Reader::get_root(mask.binary.as_slice())
// 		.map_err(|_| anyhow!("Mask is not a valid flexbuffer"))?
// 		.get_vector()?
// 		.index(5)?
// 		.get_map()
// 		.map_err(|_| anyhow!("Mask is not a valid map"))
// }
pub fn create_pulse_receiver_flex(
	_node: &Node,
	calling_client: Arc<Client>,
	data: &[u8],
) -> Result<()> {
	let root = flexbuffers::Reader::get_root(data)?;
	let flex_vec = root.get_vector()?;
	let node = Node::create("/data/receiver", flex_vec.idx(0).get_str()?, true);
	let parent = get_spatial_parent_flex(&calling_client, flex_vec.idx(1).get_str()?)?;
	let transform = get_transform_pose_flex(&flex_vec.idx(2), &flex_vec.idx(3))?;
	let field = calling_client
		.scenegraph
		.get_node(flex_vec.idx(4).as_str())
		.ok_or_else(|| anyhow!("Field not found"))?
		.field
		.get()
		.ok_or_else(|| anyhow!("Field node is not a field"))?
		.clone();

	let node_rc = calling_client.scenegraph.add_node(node);
	Spatial::add_to(&node_rc, Some(parent), transform)?;
	PulseReceiver::add_to(&node_rc, field)?;
	Ok(())
}

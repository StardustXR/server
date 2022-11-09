use super::{find_spatial, Spatial, ZONEABLE_REGISTRY};
use crate::{
	core::{client::Client, registry::Registry},
	nodes::{
		alias::{Alias, AliasInfo},
		fields::{find_field, Field},
		spatial::{find_spatial_parent, parse_transform},
		Node,
	},
};
use anyhow::Result;
use glam::vec3a;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use serde::Deserialize;
use stardust_xr::{
	schemas::flex::{deserialize, serialize},
	values::Transform,
};
use std::sync::{Arc, Weak};

pub fn capture(spatial: &Arc<Spatial>, zone: &Arc<Zone>) {
	let old_distance = spatial.zone_distance();
	let new_distance = zone
		.field
		.upgrade()
		.map(|field| field.distance(spatial, vec3a(0.0, 0.0, 0.0)))
		.unwrap_or(f32::MAX);
	if new_distance.abs() < old_distance.abs() {
		release(spatial);
		*spatial.old_parent.lock() = spatial.parent.lock().clone();
		*spatial.zone.lock() = Arc::downgrade(zone);
		zone.captured.add_raw(spatial);
		let node = zone.spatial.node.upgrade().unwrap();
		let _ = node.send_remote_signal("capture", &serialize(&spatial.uid).unwrap());
	}
}
pub fn release(spatial: &Spatial) {
	let _ = spatial.set_spatial_parent_in_place(spatial.old_parent.lock().take().as_ref());
	let mut spatial_zone = spatial.zone.lock();
	if let Some(spatial_zone) = spatial_zone.upgrade() {
		let node = spatial_zone.spatial.node.upgrade().unwrap();
		spatial_zone.captured.remove(spatial);
		let _ = node.send_remote_signal("release", &serialize(&spatial.uid).unwrap());
	}
	*spatial_zone = Weak::new();
}

pub struct Zone {
	spatial: Arc<Spatial>,
	pub field: Weak<Field>,
	zoneables: Mutex<FxHashMap<String, Arc<Node>>>,
	captured: Registry<Spatial>,
}
impl Zone {
	pub fn add_to(node: &Arc<Node>, spatial: Arc<Spatial>, field: &Arc<Field>) -> Arc<Zone> {
		let zone = Arc::new(Zone {
			spatial,
			field: Arc::downgrade(field),
			zoneables: Mutex::new(FxHashMap::default()),
			captured: Registry::new(),
		});
		node.add_local_signal("capture", Zone::capture_flex);
		node.add_local_signal("release", Zone::release_flex);
		node.add_local_signal("update", Zone::update);
		let _ = node.zone.set(zone.clone());
		zone
	}
	fn capture_flex(node: &Node, calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		let zone = node.zone.get().unwrap();
		let capture_uid: &str = deserialize(data)?;
		let capture_path = node.path.clone() + "/" + capture_uid;
		let spatial = find_spatial(&calling_client, "Spatial", &capture_path)?;
		capture(&spatial, zone);
		Ok(())
	}
	fn release_flex(node: &Node, calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
		let capture_uid: &str = deserialize(data)?;
		let capture_path = node.path.clone() + "/" + capture_uid;
		let spatial = find_spatial(&calling_client, "Spatial", &capture_path)?;
		release(&spatial);
		Ok(())
	}
	fn update(node: &Node, _calling_client: Arc<Client>, _data: &[u8]) -> Result<()> {
		let zone = node.zone.get().unwrap();
		if let Some(field) = zone.field.upgrade() {
			if let Some((zone_client, zone_node)) = zone
				.spatial
				.node
				.upgrade()
				.and_then(|n| n.get_client().zip(Some(n)))
			{
				let mut old_zoneables = zone.zoneables.lock();
				for (_uid, zoneable) in old_zoneables.iter() {
					zoneable.destroy();
				}
				let captured = zone.captured.get_valid_contents();
				let zoneables = ZONEABLE_REGISTRY
					.get_valid_contents()
					.into_iter()
					.filter(|zoneable| zoneable.node.upgrade().is_some())
					.filter(|zoneable| {
						if captured
							.iter()
							.any(|captured| Arc::ptr_eq(captured, zoneable))
						{
							return true;
						}
						let spatial_zone_distance = zoneable.zone_distance();
						let self_zone_distance = field.distance(zoneable, vec3a(0.0, 0.0, 0.0));
						self_zone_distance < 0.0 && spatial_zone_distance > self_zone_distance
					})
					.map(|zoneable| {
						let alias = Alias::create(
							&zone_client,
							zone_node.get_path(),
							&zoneable.uid,
							&zoneable.node.upgrade().unwrap(),
							AliasInfo {
								local_signals: vec![
									"set_transform",
									"set_spatial_parent",
									"set_spatial_parent_in_place",
								],
								local_methods: vec!["get_transform"],
								..Default::default()
							},
						);
						(zoneable.uid.clone(), alias)
					})
					.collect::<FxHashMap<String, Arc<Node>>>();

				for entered_uid in zoneables.keys().filter(|k| !old_zoneables.contains_key(*k)) {
					node.send_remote_signal("enter", &serialize(entered_uid)?)?;
				}
				for left_uid in old_zoneables.keys().filter(|k| !zoneables.contains_key(*k)) {
					node.send_remote_signal("leave", &serialize(left_uid)?)?;
				}

				*old_zoneables = zoneables;

				Ok(())
			} else {
				Err(anyhow::anyhow!("No client on node?"))
			}
		} else {
			Err(anyhow::anyhow!("Zone's field has been destroyed"))
		}
	}
}
impl Drop for Zone {
	fn drop(&mut self) {
		for captured in self.captured.get_valid_contents() {
			release(&captured);
		}
	}
}

pub fn create_zone_flex(_node: &Node, calling_client: Arc<Client>, data: &[u8]) -> Result<()> {
	#[derive(Deserialize)]
	struct CreateZoneInfo<'a> {
		name: &'a str,
		parent_path: &'a str,
		transform: Transform,
		field_path: &'a str,
	}
	let info: CreateZoneInfo = deserialize(data)?;
	let parent = find_spatial_parent(&calling_client, info.parent_path)?;
	let transform = parse_transform(info.transform, true, true, false)?;
	let field = find_field(&calling_client, info.field_path)?;

	let node = Node::create(&calling_client, "/spatial/zone", info.name, true).add_to_scenegraph();
	let space = Spatial::add_to(&node, Some(parent), transform, false)?;
	Zone::add_to(&node, space, &field);
	Ok(())
}

use super::{Spatial, ZoneAspect, ZONEABLE_REGISTRY};
use crate::{
	core::{client::Client, registry::Registry},
	nodes::{
		alias::{Alias, AliasInfo},
		fields::Field,
		Aspect, Node,
	},
};
use glam::vec3a;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
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
		*spatial.old_parent.lock() = spatial.get_parent();
		*spatial.zone.lock() = Arc::downgrade(zone);
		zone.captured.add_raw(spatial);
		let Some(node) = zone.spatial.node.upgrade() else {
			return;
		};
		let _ = super::zone_client::capture(&node, &spatial.uid);
	}
}
pub fn release(spatial: &Arc<Spatial>) {
	let _ = spatial.set_spatial_parent_in_place(spatial.old_parent.lock().take().as_ref());
	let mut spatial_zone = spatial.zone.lock();
	if let Some(spatial_zone) = spatial_zone.upgrade() {
		let Some(node) = spatial_zone.spatial.node.upgrade() else {
			return;
		};
		spatial_zone.captured.remove(spatial);
		let _ = super::zone_client::release(&node, &spatial.uid);
	}
	*spatial_zone = Weak::new();
}
pub(super) fn release_drop(spatial: &Spatial) {
	let spatial_zone = spatial.zone.lock();
	if let Some(spatial_zone) = spatial_zone.upgrade() {
		let Some(node) = spatial_zone.spatial.node.upgrade() else {
			return;
		};
		spatial_zone.captured.remove(spatial);
		let _ = super::zone_client::release(&node, &spatial.uid);
	}
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
		<Zone as ZoneAspect>::add_node_members(node);
		node.add_aspect_raw(zone.clone());
		zone
	}
}
impl Aspect for Zone {
	const NAME: &'static str = "Zone";
}
impl ZoneAspect for Zone {
	fn update(node: Arc<Node>, _calling_client: Arc<Client>) -> color_eyre::eyre::Result<()> {
		let zone = node.get_aspect::<Zone>()?;
		let Some(field) = zone.field.upgrade() else {
			return Err(color_eyre::eyre::eyre!("Zone's field has been destroyed"));
		};
		let Some((zone_client, zone_node)) = zone
			.spatial
			.node
			.upgrade()
			.and_then(|n| n.get_client().zip(Some(n)))
		else {
			return Err(color_eyre::eyre::eyre!("No client on node?"));
		};
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
			.filter_map(|zoneable| {
				let alias = Alias::create(
					&zone_client,
					zone_node.get_path(),
					&zoneable.uid,
					&zoneable.node.upgrade().unwrap(),
					AliasInfo {
						server_signals: vec![
							"set_transform",
							"set_spatial_parent",
							"set_spatial_parent_in_place",
						],
						server_methods: vec!["get_bounds", "get_transform"],
						..Default::default()
					},
				)
				.ok()?;
				Some((zoneable.uid.clone(), alias))
			})
			.collect::<FxHashMap<String, Arc<Node>>>();

		for (uid, zoneable) in zoneables
			.iter()
			.filter(|(k, _)| !old_zoneables.contains_key(*k))
		{
			super::zone_client::enter(&node, uid, zoneable)?;
		}
		for left_uid in old_zoneables.keys().filter(|k| !zoneables.contains_key(*k)) {
			super::zone_client::leave(&node, &left_uid)?;
		}

		*old_zoneables = zoneables;

		Ok(())
	}

	fn capture(
		node: Arc<Node>,
		_calling_client: Arc<Client>,
		spatial: Arc<Node>,
	) -> color_eyre::eyre::Result<()> {
		let zone = node.get_aspect::<Zone>()?;
		let spatial = spatial.get_aspect()?;
		capture(&spatial, &zone);
		Ok(())
	}

	fn release(
		_node: Arc<Node>,
		_calling_client: Arc<Client>,
		spatial: Arc<Node>,
	) -> color_eyre::eyre::Result<()> {
		let spatial = spatial.get_aspect()?;
		release(&spatial);
		Ok(())
	}
}
impl Drop for Zone {
	fn drop(&mut self) {
		for captured in self.captured.get_valid_contents() {
			release(&captured);
		}
	}
}

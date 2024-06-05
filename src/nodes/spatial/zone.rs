use super::{
	Spatial, ZoneAspect, SPATIAL_ASPECT_ALIAS_INFO, SPATIAL_REF_ASPECT_ALIAS_INFO,
	ZONEABLE_REGISTRY,
};
use crate::{
	core::{client::Client, registry::Registry},
	nodes::{
		alias::{get_original, Alias, AliasList},
		fields::Field,
		Aspect, Node,
	},
};
use color_eyre::eyre::Result;
use glam::vec3a;
use std::sync::{Arc, Weak};

pub fn capture(spatial: &Arc<Spatial>, zone: &Arc<Zone>) {
	let old_distance = spatial.zone_distance();
	let new_distance = zone.field.distance(spatial, vec3a(0.0, 0.0, 0.0));
	if new_distance.abs() < old_distance.abs() {
		release(spatial);
		*spatial.old_parent.lock() = spatial.get_parent();
		*spatial.zone.lock() = Arc::downgrade(zone);
		let Some(zone_node) = zone.spatial.node.upgrade() else {
			return;
		};
		let Some(spatial_node) = spatial.node.upgrade() else {
			return;
		};
		let Ok(spatial_alias) = Alias::create(
			&spatial_node,
			&zone_node.get_client().unwrap(),
			SPATIAL_ASPECT_ALIAS_INFO.clone(),
			Some(&zone.captured),
		) else {
			return;
		};
		let _ = super::zone_client::capture(&zone_node, &spatial_alias);
	}
}
pub fn release(spatial: &Spatial) {
	let Some(spatial_node) = spatial.node.upgrade() else {
		return;
	};
	let spatial = spatial_node.get_aspect::<Spatial>().unwrap();

	let _ = spatial.set_spatial_parent_in_place(spatial.old_parent.lock().take().as_ref());
	let mut spatial_zone = spatial.zone.lock();

	if let Some(spatial_zone) = spatial_zone.upgrade() {
		spatial_zone.captured.remove_aspect(spatial.as_ref());
		let Some(node) = spatial_zone.spatial.node.upgrade() else {
			return;
		};
		let _ = super::zone_client::release(&node, spatial_node.id);
	}
	*spatial_zone = Weak::new();
}

pub struct Zone {
	spatial: Arc<Spatial>,
	pub field: Arc<Field>,
	intersecting_spatials: Registry<Spatial>,
	intersecting: AliasList,
	captured: AliasList,
}
impl Zone {
	pub fn add_to(node: &Arc<Node>, spatial: Arc<Spatial>, field: Arc<Field>) -> Arc<Zone> {
		let zone = Arc::new(Zone {
			spatial,
			field,
			intersecting_spatials: Registry::default(),
			intersecting: AliasList::default(),
			captured: AliasList::default(),
		});
		<Zone as ZoneAspect>::add_node_members(node);
		node.add_aspect_raw(zone.clone());
		zone
	}
	pub fn update(&self) -> Result<()> {
		let node = self.spatial.node().unwrap();

		let current_zoneables = Registry::new();
		for zoneable in ZONEABLE_REGISTRY.get_valid_contents() {
			let distance = self.field.distance(&zoneable, [0.0; 3].into());
			if distance > 0.0 {
				continue;
			}
			if let Some(zone) = zoneable.zone.lock().upgrade() {
				let zoneable_distance = zone.field.distance(&zoneable, [0.0; 3].into());
				if zoneable_distance < distance {
					continue;
				}
			}
			current_zoneables.add_raw(&zoneable);
		}

		let (added, removed) =
			Registry::get_changes(&self.intersecting_spatials, &current_zoneables);
		for added in added {
			let Some(added_node) = added.node() else {
				continue;
			};
			let Ok(alias) = Alias::create(
				&added_node,
				&self.spatial.node().unwrap().get_client().unwrap(),
				SPATIAL_REF_ASPECT_ALIAS_INFO.clone(),
				Some(&self.intersecting),
			) else {
				continue;
			};
			let _ = super::zone_client::enter(&node, &alias);
		}
		for removed in removed {
			let Some(removed_node) = removed.node() else {
				continue;
			};
			release(&removed);
			let _ = super::zone_client::leave(&node, removed_node.id);
			self.intersecting.remove_aspect(removed.as_ref());
		}
		self.intersecting_spatials.set(&current_zoneables);

		Ok(())
	}
}
impl Aspect for Zone {
	const NAME: &'static str = "Zone";
}
impl ZoneAspect for Zone {
	fn update(node: Arc<Node>, _calling_client: Arc<Client>) -> Result<()> {
		let zone = node.get_aspect::<Zone>()?;
		let _ = zone.update();
		Ok(())
	}

	fn capture(node: Arc<Node>, _calling_client: Arc<Client>, spatial: Arc<Node>) -> Result<()> {
		let zone = node.get_aspect::<Zone>()?;
		let spatial = spatial.get_aspect()?;
		capture(&spatial, &zone);
		Ok(())
	}

	fn release(_node: Arc<Node>, _calling_client: Arc<Client>, spatial: Arc<Node>) -> Result<()> {
		let spatial = spatial.get_aspect()?;
		release(&spatial);
		Ok(())
	}
}
impl Drop for Zone {
	fn drop(&mut self) {
		for captured in self
			.captured
			.get_aliases()
			.into_iter()
			.filter_map(get_original)
			.filter_map(|n| n.get_aspect::<Spatial>().ok())
		{
			release(&captured);
		}
	}
}

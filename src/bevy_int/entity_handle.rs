use std::ops::Deref;
use std::sync::Arc;

use bevy::prelude::*;

use crate::nodes::spatial::SpatialNode;

use super::bevy_channel::{BevyChannel, BevyChannelReader};
pub struct EntityHandlePlugin;

impl Plugin for EntityHandlePlugin {
	fn build(&self, app: &mut App) {
		DESTROY.init(app);
		app.add_systems(PreUpdate, despawn);
	}
}

fn despawn(
	mut cmds: Commands,
	mut reader: ResMut<BevyChannelReader<Entity>>,
	child_query: Query<&Children>,
	has_spatial: Query<Has<SpatialNode>>,
) {
	while let Some(e) = reader.read() {
		if let Ok(children) = child_query.get(e) {
			for e in children {
				if has_spatial.get(*e).unwrap_or_default() {
					cmds.entity(*e).try_remove::<ChildOf>();
				}
			}
		}
		cmds.entity(e).despawn();
	}
}

#[derive(Debug, Hash, PartialEq, Eq, PartialOrd, Ord, Clone)]
pub struct EntityHandle(Arc<EntityHandleInner>);
impl EntityHandle {
	pub fn get(&self) -> Entity {
		self.0.0
	}
	pub fn new(entity: Entity) -> Self {
		Self(EntityHandleInner(entity).into())
	}
}
impl Deref for EntityHandle {
	type Target = Entity;

	fn deref(&self) -> &Self::Target {
		&self.0.0
	}
}
static DESTROY: BevyChannel<Entity> = BevyChannel::new();
#[derive(Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct EntityHandleInner(Entity);
impl Drop for EntityHandleInner {
	fn drop(&mut self) {
		DESTROY.send(self.0);
	}
}

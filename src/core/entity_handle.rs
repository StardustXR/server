use std::ops::Deref;
use std::sync::Arc;

use bevy::prelude::*;

use super::bevy_channel::{BevyChannel, BevyChannelReader};
pub struct EntityHandlePlugin;

impl Plugin for EntityHandlePlugin {
	fn build(&self, app: &mut App) {
		DESTROY.init(app);
		app.add_systems(PreUpdate, despawn);
	}
}

fn despawn(mut cmds: Commands, mut reader: ResMut<BevyChannelReader<Entity>>) {
	while let Some(e) = reader.read() {
		cmds.entity(e).try_despawn();
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

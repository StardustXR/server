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

static DESTROY: BevyChannel<Entity> = BevyChannel::new();
#[derive(Deref, DerefMut, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct EntityHandle(pub Entity);
impl Drop for EntityHandle {
	fn drop(&mut self) {
		DESTROY.send(self.0);
	}
}
impl From<Entity> for EntityHandle {
	fn from(value: Entity) -> Self {
		Self(value)
	}
}

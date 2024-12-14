use std::ops::Deref as _;

use bevy::{
	app::MainScheduleOrder, asset::embedded_asset, ecs::schedule::ScheduleLabel, prelude::*,
};
use bevy_mod_openxr::session::OxrSession;
use bevy_mod_xr::session::{session_available, XrSessionCreated};
use openxr::ReferenceSpaceType;

use crate::objects::Inputs;

pub struct StardustBevyPlugin;

impl Plugin for StardustBevyPlugin {
	fn build(&self, app: &mut App) {
		app.init_schedule(StardustExtract);
		let labels = &mut app.world_mut().resource_mut::<MainScheduleOrder>().labels;
		info!("test: {labels:?}");
		labels.insert(labels.len() - 2, StardustExtract.intern());
		app.add_systems(Startup, spawn_camera.run_if(not(session_available)));
		app.add_systems(XrSessionCreated, make_view_space);
		embedded_asset!(app, "src/objects/input", "objects/input/cursor.glb");
	}
}

fn make_view_space(mut cmds: Commands, session: Res<OxrSession>) {
	// idk what errors this function returns
	let view_space = session
		.create_reference_space(ReferenceSpaceType::VIEW, Transform::IDENTITY)
		.unwrap();
	// this locates the view space against the default reference space (stage i belive) and sets
	// the transform relative to the XrTrackingRoot
	cmds.spawn((view_space.0, ViewLocation));
}

fn spawn_camera(mut cmds: Commands) {
	cmds.spawn((Camera3d::default(), ViewLocation));
}

#[derive(Deref, DerefMut, Resource)]
pub struct BevyToStardustEvents(pub Vec<BevyToStardustEvent>);
pub enum BevyToStardustEvent {
	InputsCreated(Inputs),
	SessionDestroyed,
	SessionEnding,
	SessionCreated(OxrSession),
	MainSessionVisible(bool),
}

#[derive(ScheduleLabel, Hash, Debug, Clone, Copy, PartialEq, Eq)]
pub struct StardustExtract;
#[derive(Component, Hash, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TemporaryEntity;
#[derive(Component, Hash, Debug, Clone, Copy, PartialEq, Eq)]
#[require(GlobalTransform)]
pub struct ViewLocation;
#[derive(Hash, Debug, Clone, Copy, PartialEq, Eq, Deref)]
pub struct MainWorldEntity(Entity);

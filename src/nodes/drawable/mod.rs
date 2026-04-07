pub mod dmatex;
pub mod lines;
pub mod model;
pub mod sky;
// pub mod text;

#[derive(bevy::ecs::schedule::SystemSet, Hash, Debug, PartialEq, Eq, Clone, Copy)]
pub struct ModelNodeSystemSet;

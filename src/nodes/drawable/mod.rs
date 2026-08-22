pub mod dmatex;
// FIX ORDER: 2
// pub mod lines;
// FIX ORDER: 2
// pub mod model;
// FIX ORDER: 1
// pub mod sky;
// FIX ORDER: 3
// pub mod text;

#[derive(bevy::ecs::schedule::SystemSet, Hash, Debug, PartialEq, Eq, Clone, Copy)]
pub struct ModelNodeSystemSet;

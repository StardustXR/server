use bevy::asset::Assets;
use bevy::ecs::world::Mut;
use bevy::image::Image;

pub struct GraphicsInfo<'w> {
	pub _images: Mut<'w, Assets<Image>>,
}

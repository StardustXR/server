use bevy::{
	app::{Plugin, Update},
	color::Color,
	core_pipeline::{Skybox, core_3d::Camera3d},
	ecs::{
		entity::Entity,
		query::With,
		system::{Commands, Query, ResMut},
	},
	pbr::{AmbientLight, environment_map::EnvironmentMapLight},
};
use bevy_equirect::EquirectManager;
use glam::Quat;

pub struct SkyPlugin;

impl Plugin for SkyPlugin {
	fn build(&self, app: &mut bevy::app::App) {
		app.add_systems(Update, apply_sky);
	}
}

// TODO: make this work with cameras spawned after setting the sky texture
fn apply_sky(
	mut equirect: ResMut<EquirectManager>,
	mut ambient_light: ResMut<AmbientLight>,
	cameras: Query<Entity, With<Camera3d>>,
	mut cmds: Commands,
) {
	if let Some(tex) = super::QUEUED_SKYTEX.lock().take() {
		if let Some(path) = tex {
			let image_handle = equirect.load_equirect_as_cubemap(path, 2048);
			for cam in cameras {
				cmds.entity(cam).insert(Skybox {
					image: image_handle.clone(),
					brightness: 1000.0,
					rotation: Quat::IDENTITY,
				});
			}
		} else {
			for cam in cameras {
				cmds.entity(cam).remove::<Skybox>();
			}
		}
	}
	if let Some(light) = super::QUEUED_SKYLIGHT.lock().take() {
		if let Some(path) = light {
			let image_handle = equirect.load_equirect_as_cubemap(path, 2048);
			for cam in cameras {
				cmds.entity(cam).insert(EnvironmentMapLight {
					diffuse_map: image_handle.clone(),
					// we might want to use the SkyTex for this?
					specular_map: image_handle.clone(),
					intensity: 1000.0,
					rotation: Quat::IDENTITY,
					affects_lightmapped_mesh_diffuse: false,
				});
			}
			ambient_light.color = Color::BLACK;
		} else {
			for cam in cameras {
				cmds.entity(cam).remove::<EnvironmentMapLight>();
			}
			ambient_light.color = Color::WHITE;
		}
	}
}

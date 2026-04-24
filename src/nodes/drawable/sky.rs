use std::{
	ffi::OsStr,
	path::PathBuf,
	sync::atomic::{AtomicBool, Ordering},
};

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
use gluon_wire::impl_transaction_handler;
use parking_lot::Mutex;
use stardust_xr_protocol::{
	sky::{SkyGuard as SkyGuardProxy, SkyGuardHandler, SkyInterfaceHandler},
	types::Resource,
};
use stardust_xr_server_foundation::resource::get_resource_file;

use crate::{PION, interface};

pub struct SkyPlugin;

impl Plugin for SkyPlugin {
	fn build(&self, app: &mut bevy::app::App) {
		app.add_systems(Update, apply_sky);
	}
}

static QUEUED_SKYLIGHT: Mutex<Option<Option<PathBuf>>> = Mutex::new(None);
static QUEUED_SKYTEX: Mutex<Option<Option<PathBuf>>> = Mutex::new(None);
static SKYLIGHT_SET: AtomicBool = AtomicBool::new(false);
static SKYTEX_SET: AtomicBool = AtomicBool::new(false);

// TODO: make this work with cameras spawned after setting the sky texture
fn apply_sky(
	mut equirect: ResMut<EquirectManager>,
	mut ambient_light: ResMut<AmbientLight>,
	cameras: Query<Entity, With<Camera3d>>,
	mut cmds: Commands,
) {
	if let Some(tex) = QUEUED_SKYTEX.lock().take() {
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
	if let Some(light) = QUEUED_SKYLIGHT.lock().take() {
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

interface!(SkyInterface);
impl SkyInterfaceHandler for SkyInterface {
	async fn set_sky_tex(
		&self,
		_ctx: gluon_wire::GluonCtx,
		tex: Resource,
	) -> Option<SkyGuardProxy> {
		if SKYTEX_SET.load(Ordering::Relaxed) {
			return None;
		}
		let resource_path = get_resource_file(
			&tex,
			self.base_prefixes(),
			&[OsStr::new("hdr"), OsStr::new("png"), OsStr::new("jpg")],
		)?;
		QUEUED_SKYTEX.lock().replace(Some(resource_path));
		SKYTEX_SET.store(true, Ordering::Relaxed);
		let guard = PION.register_object(SkyGuard { is_sky_tex: true });
		let proxy = SkyGuardProxy::from_handler(&guard);
		guard.to_service();
		Some(proxy)
	}

	async fn set_sky_light(
		&self,
		_ctx: gluon_wire::GluonCtx,
		tex: Resource,
	) -> Option<SkyGuardProxy> {
		if SKYLIGHT_SET.load(Ordering::Relaxed) {
			return None;
		}
		let resource_path = get_resource_file(
			&tex,
			self.base_prefixes(),
			&[OsStr::new("hdr"), OsStr::new("png"), OsStr::new("jpg")],
		)?;
		QUEUED_SKYLIGHT.lock().replace(Some(resource_path));
		SKYLIGHT_SET.store(true, Ordering::Relaxed);
		let guard = PION.register_object(SkyGuard { is_sky_tex: false });
		let proxy = SkyGuardProxy::from_handler(&guard);
		guard.to_service();
		Some(proxy)
	}
}

#[derive(Debug)]
struct SkyGuard {
	is_sky_tex: bool,
}
impl SkyGuardHandler for SkyGuard {}
impl_transaction_handler!(SkyGuard);
impl Drop for SkyGuard {
	fn drop(&mut self) {
		if self.is_sky_tex {
			QUEUED_SKYTEX.lock().replace(None);
		} else {
			QUEUED_SKYLIGHT.lock().replace(None);
		}
	}
}

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
		observer::Trigger,
		query::With,
		resource::Resource,
		system::{Commands, Query, Res, ResMut},
		world::OnInsert,
	},
	pbr::{AmbientLight, environment_map::EnvironmentMapLight},
};
use bevy_equirect::EquirectManager;
use bevy_mod_openxr::{
	environment_blend_mode::OxrEnvironmentBlendModes, resources::OxrSessionConfig,
};
use glam::Quat;
use gluon::Handler;
use openxr::EnvironmentBlendMode;
use parking_lot::Mutex;
use stardust_xr_protocol::{
	sky::{SkyGuard as SkyGuardProxy, SkyGuardHandler, SkyInterfaceHandler},
	types,
};
use stardust_xr_server_foundation::resource::get_resource_file;

use crate::{PION, interface};

pub struct SkyPlugin;

impl Plugin for SkyPlugin {
	fn build(&self, app: &mut bevy::app::App) {
		app.init_resource::<Sky>();
		app.add_systems(Update, apply_sky);
		app.add_observer(new_cams);
	}
}

fn new_cams(
	v: Trigger<OnInsert, Camera3d>,
	sky: Res<Sky>,
	mut cmds: Commands,
) {
    if let Some(skybox) = sky.skybox.clone() {
        cmds.entity(v.target()).insert(skybox);
    }
    if let Some(light) = sky.light.clone() {
        cmds.entity(v.target()).insert(light);
    }
}

static QUEUED_SKYLIGHT: Mutex<Option<Option<PathBuf>>> = Mutex::new(None);
static QUEUED_SKYTEX: Mutex<Option<Option<(PathBuf, bool)>>> = Mutex::new(None);
static SKYLIGHT_SET: AtomicBool = AtomicBool::new(false);
static SKYTEX_SET: AtomicBool = AtomicBool::new(false);

#[derive(Resource, Default)]
struct Sky {
	skybox: Option<Skybox>,
	light: Option<EnvironmentMapLight>,
}

fn modify_blend_modes(
	blend_modes: &mut OxrEnvironmentBlendModes,
	conf: &OxrSessionConfig,
	opaque: bool,
) {
	if opaque {
		blend_modes.set_blend_mode(EnvironmentBlendMode::OPAQUE);
		return;
	}
	for pref in conf.blend_mode_preference.iter() {
		if blend_modes.set_blend_mode(*pref) {
			return;
		}
	}
}

// TODO: make this work with cameras spawned after setting the sky texture
fn apply_sky(
	mut equirect: ResMut<EquirectManager>,
	mut ambient_light: ResMut<AmbientLight>,
	mut sky: ResMut<Sky>,
	mut blend_modes: ResMut<OxrEnvironmentBlendModes>,
	session_conf: Res<OxrSessionConfig>,
	cameras: Query<Entity, With<Camera3d>>,
	mut cmds: Commands,
) {
	if let Some(tex) = QUEUED_SKYTEX.lock().take() {
		if let Some((path, opaque)) = tex {
			let image_handle = equirect.load_equirect_as_cubemap(path, 2048);
			let skybox = Skybox {
				image: image_handle.clone(),
				brightness: 1000.0,
				rotation: Quat::IDENTITY,
			};
			for cam in cameras {
				cmds.entity(cam).insert(skybox.clone());
			}
			sky.skybox.replace(skybox);
			modify_blend_modes(&mut blend_modes, &session_conf, opaque);
		} else {
			for cam in cameras {
				cmds.entity(cam).remove::<Skybox>();
			}
			sky.skybox.take();
			modify_blend_modes(&mut blend_modes, &session_conf, false);
		}
	}
	if let Some(light) = QUEUED_SKYLIGHT.lock().take() {
		if let Some(path) = light {
			let image_handle = equirect.load_equirect_as_cubemap(path, 2048);
			let light = EnvironmentMapLight {
				diffuse_map: image_handle.clone(),
				// we might want to use the SkyTex for this?
				specular_map: image_handle.clone(),
				intensity: 1000.0,
				rotation: Quat::IDENTITY,
				affects_lightmapped_mesh_diffuse: false,
			};
			for cam in cameras {
				cmds.entity(cam).insert(light.clone());
			}
			ambient_light.color = Color::BLACK;
			sky.light.replace(light);
		} else {
			for cam in cameras {
				cmds.entity(cam).remove::<EnvironmentMapLight>();
			}
			ambient_light.color = Color::WHITE;
			sky.light.take();
		}
	}
}

interface!(SkyInterface);
impl SkyInterfaceHandler for SkyInterface {
	async fn set_sky_tex(
		&self,
		_ctx: gluon::Context,
		tex: types::Resource,
		opaque: bool,
	) -> Option<SkyGuardProxy> {
		// TODO: actually use opaque
		if SKYTEX_SET.load(Ordering::Relaxed) {
			return None;
		}
		let resource_path = get_resource_file(
			&tex,
			self.base_prefixes(),
			&[OsStr::new("hdr"), OsStr::new("png"), OsStr::new("jpg")],
		)?;
		QUEUED_SKYTEX.lock().replace(Some((resource_path, opaque)));
		SKYTEX_SET.store(true, Ordering::Relaxed);
		let guard = PION.register_object(SkyGuard { is_sky_tex: true });
		Some(SkyGuardProxy::from_handler(&guard.to_service()))
	}

	async fn set_sky_light(
		&self,
		_ctx: gluon::Context,
		tex: types::Resource,
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
		Some(SkyGuardProxy::from_handler(&guard.to_service()))
	}
}

#[derive(Debug, Handler)]
struct SkyGuard {
	is_sky_tex: bool,
}
impl SkyGuardHandler for SkyGuard {}
impl Drop for SkyGuard {
	fn drop(&mut self) {
		if self.is_sky_tex {
			QUEUED_SKYTEX.lock().replace(None);
		} else {
			QUEUED_SKYLIGHT.lock().replace(None);
		}
	}
}

use super::{get_sorted_handlers, CaptureManager};
use crate::{
	bevy_plugin::{DbusConnection, InputUpdate},
	core::client::INTERNAL_CLIENT,
	nodes::{
		fields::{Field, FieldTrait},
		input::{InputDataType, InputHandler, InputMethod, Tip, INPUT_HANDLER_REGISTRY},
		spatial::Spatial,
		Node, OwnedNode,
	},
	objects::{ObjectHandle, SpatialRef},
	DefaultMaterial,
};
use bevy::{
	app::{App, Plugin},
	asset::{embedded_asset, AssetServer, Assets, Handle},
	color::LinearRgba,
	gltf::GltfAssetLabel,
	pbr::MeshMaterial3d,
	prelude::{Children, Commands, Component, Mesh, Query, Res, ResMut, Transform},
	scene::SceneRoot,
};
use bevy_mod_openxr::{
	helper_traits::{ToQuat, ToVec2, ToVec3},
	resources::OxrFrameState,
	session::OxrSession,
	spaces::{OxrSpaceExt, OxrSpaceLocationFlags},
};
use bevy_mod_xr::{
	hands::HandSide,
	session::XrSessionCreated,
	spaces::{XrPrimaryReferenceSpace, XrSpace},
};
use color_eyre::eyre::Result;
use glam::{Mat4, Vec2, Vec3};
use once_cell::sync::OnceCell;
use openxr::{ActionSet, Posef};
use serde::{Deserialize, Serialize};
use stardust_xr::values::Datamap;
use std::{ops::Deref, sync::Arc};
use tracing::error;
use zbus::Connection;

#[derive(Default, Debug, Deserialize, Serialize)]
struct ControllerDatamap {
	select: f32,
	middle: f32,
	context: f32,
	grab: f32,
	scroll: Vec2,
}

pub struct StardustControllerPlugin;
impl Plugin for StardustControllerPlugin {
	fn build(&self, app: &mut App) {
		embedded_asset!(app, "src/objects/input", "cursor.glb");
		app.add_systems(XrSessionCreated, spawn_controllers);
		app.add_systems(InputUpdate, update_controllers);
	}
}

fn update_controllers(
	mut mats: ResMut<Assets<DefaultMaterial>>,
	mut query: Query<(&mut SkController, &mut Transform)>,
	time: Res<OxrFrameState>,
	base_space: Res<XrPrimaryReferenceSpace>,
	session: ResMut<OxrSession>,
) {
	for (mut controller, mut transform) in query.iter_mut() {
		let input_node = controller.input.spatial.node().unwrap();
		let location = (|| {
			let location = match session.locate_space(
				&XrSpace::from_raw_openxr_space(controller.space.as_raw()),
				&base_space,
				time.predicted_display_time,
			) {
				Err(err) => {
					error!("issues locating controller space: {err}");
					return None;
				}
				Ok(val) => val,
			};
			let flags = OxrSpaceLocationFlags(location.location_flags);

			input_node.set_enabled(flags.pos_tracked() && flags.rot_tracked());
			if flags.pos_valid() && flags.rot_valid() {
				Some(Mat4::from_rotation_translation(
					location.pose.orientation.to_quat(),
					location.pose.position.to_vec3(),
				))
			} else {
				None
			}
		})()
		.unwrap_or(Mat4::IDENTITY);
		if input_node.enabled() {
			let world_transform = location;
			if let Some(mat) = controller.material.get().and_then(|v| mats.get_mut(v)) {
				mat.base_color = if controller.capture.is_none() {
					LinearRgba::rgb(1.0, 1.0, 1.0)
				} else {
					LinearRgba::rgb(0.0, 1.0, 0.75)
				}
				.into();
			}

			*transform =
				Transform::from_matrix(world_transform * Mat4::from_scale(Vec3::ONE * 0.02));
			controller
				.input
				.spatial
				.set_local_transform(world_transform);
		}
		controller.datamap.select = controller
			.actions
			.trigger
			.state(&session, openxr::Path::NULL)
			.map(|v| v.current_state)
			.unwrap_or_default();
		controller.datamap.grab = controller
			.actions
			.grip
			.state(&session, openxr::Path::NULL)
			.map(|v| v.current_state)
			.unwrap_or_default();
		controller.datamap.scroll = controller
			.actions
			.stick
			.state(&session, openxr::Path::NULL)
			.map(|v| v.current_state.to_vec2())
			.unwrap_or_default();
		*controller.input.datamap.lock() = Datamap::from_typed(&controller.datamap).unwrap();

		// remove the capture when it's removed from captures list
		if let Some(capture) = &controller.capture {
			if !controller
				.input
				.capture_requests
				.get_valid_contents()
				.contains(capture)
			{
				controller.capture.take();
			}
		}
		// add the capture that's the closest if we don't have one
		if controller.capture.is_none() {
			controller.capture = controller
				.input
				.capture_requests
				.get_valid_contents()
				.into_iter()
				.map(|handler| {
					(
						handler.clone(),
						handler
							.field
							.distance(&controller.input.spatial, [0.0; 3].into())
							.abs(),
					)
				})
				.reduce(|(handlers_a, distance_a), (handlers_b, distance_b)| {
					if distance_a < distance_b {
						(handlers_a, distance_a)
					} else {
						(handlers_b, distance_b)
					}
				})
				.map(|(rx, _)| rx);
		}

		// make sure that if something is captured only send input to it
		controller.input.captures.clear();
		if let Some(capture) = &controller.capture {
			controller.input.set_handler_order([capture].into_iter());
			controller.input.captures.add_raw(capture);
			return;
		}

		// send input to all the input handlers that are the closest to the ray as possible
		controller.input.set_handler_order(
			INPUT_HANDLER_REGISTRY
				.get_valid_contents()
				.into_iter()
				// filter out all the disabled handlers
				.filter(|handler| {
					let Some(node) = handler.spatial.node() else {
						return false;
					};
					node.enabled()
				})
				// filter out all the fields with disabled handlers
				.filter(|handler| {
					let Some(node) = handler.field.spatial.node() else {
						return false;
					};
					node.enabled()
				})
				// get the unsigned distance to the handler's field (unsigned so giant fields won't always eat input)
				.map(|handler| {
					(
						vec![handler.clone()],
						handler
							.field
							.distance(&controller.input.spatial, [0.0; 3].into())
							.abs(),
					)
				})
				// now collect all handlers that are same distance if they're the closest
				.reduce(|(mut handlers_a, distance_a), (handlers_b, distance_b)| {
					if (distance_a - distance_b).abs() < 0.001 {
						// distance is basically the same (within 1mm)
						handlers_a.extend(handlers_b);
						(handlers_a, distance_a)
					} else if distance_a < distance_b {
						(handlers_a, distance_a)
					} else {
						(handlers_b, distance_b)
					}
				})
				.map(|(rx, _)| rx)
				.unwrap_or_default()
				.iter(),
		);
	}
}

fn spawn_controllers(
	connection: Res<DbusConnection>,
	asset_server: Res<AssetServer>,
	session: Res<OxrSession>,
	mut cmds: Commands,
) {
	let handle = asset_server.load(GltfAssetLabel::Scene(0).from_asset("embedded://cursor.glb"));
	for handed in [HandSide::Left, HandSide::Right] {
		let side = match handed {
			HandSide::Left => "left",
			HandSide::Right => "right",
		};
		let (spatial, object_handle) = SpatialRef::create(
			&connection,
			&("/org/stardustxr/Controller/".to_string() + side),
		);
		let tip = InputDataType::Tip(Tip::default());
		let Ok(input) = (|| -> color_eyre::Result<Arc<InputMethod>> {
			Ok(InputMethod::add_to(
				&spatial.node().unwrap(),
				tip,
				Datamap::from_typed(ControllerDatamap::default())?,
			)?)
		})() else {
			continue;
		};
		let actions = {
			let set = session
				.instance()
				.create_action_set(
					&format!("controller-{side}"),
					&format!("{side} controller"),
					0,
				)
				.unwrap();
			Actions {
				set: set.clone(),
				trigger: set
					.create_action(&format!("trigger-{side}"), &format!("{side} trigger"), &[])
					.unwrap(),
				grip: set
					.create_action(&format!("grip-{side}"), &format!("{side} grip"), &[])
					.unwrap(),
				stick: set
					.create_action(&format!("stick-{side}"), &format!("{side} stick"), &[])
					.unwrap(),
				pose: set
					.create_action(&format!("pose-{side}"), &format!("{side} pose"), &[])
					.unwrap(),
			}
		};
		cmds.spawn((
			SceneRoot(handle.clone()),
			SkController {
				object_handle,
				input,
				handed,
				material: OnceCell::new(),
				capture: None,
				datamap: Default::default(),
				space: actions
					.pose
					.create_space(
						session.deref().deref().clone(),
						openxr::Path::NULL,
						Posef::IDENTITY,
					)
					.unwrap(),
				actions,
			},
		));
	}
}

#[derive(Component)]
#[require(Transform)]
pub struct SkController {
	object_handle: ObjectHandle<SpatialRef>,
	input: Arc<InputMethod>,
	handed: HandSide,
	material: OnceCell<Handle<DefaultMaterial>>,
	capture: Option<Arc<InputHandler>>,
	datamap: ControllerDatamap,
	space: openxr::Space,
	actions: Actions,
}
struct Actions {
	set: openxr::ActionSet,
	trigger: openxr::Action<f32>,
	grip: openxr::Action<f32>,
	stick: openxr::Action<openxr::Vector2f>,
	pose: openxr::Action<openxr::Posef>,
}

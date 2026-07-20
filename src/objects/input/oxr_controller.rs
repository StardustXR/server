use crate::{
	DbusConnection, PION, PreFrameWait, get_time,
	nodes::{
		ProxyExt,
		drawable::model::{Model, ModelPart},
		fields::Field,
		spatial::{Spatial, SpatialObject, SpatialRef},
	},
	objects::{
		DebugWrapper, Tracked,
		input::{InputSender, InputSource, PointsQueryCache, QueryCache},
	},
	openxr_helpers::ConvertTimespec,
	query::spatial_query::SpatialQueryInterface,
};
use bevy::{asset::Handle, ecs::resource::Resource, tasks::futures::now_or_never};
use bevy::{math::Affine3, prelude::*};
use bevy_mod_openxr::{
	action_binding::{OxrSendActionBindings, OxrSuggestActionBinding},
	exts::OxrEnabledExtensions,
	helper_traits::{ToIsometry3d, ToQuat, ToVec2, ToVec3},
	resources::{OxrFrameState, OxrInstance, Pipelined},
	session::OxrSession,
};
use bevy_mod_xr::{
	hands::HandSide,
	session::{XrPreDestroySession, XrSessionCreated, XrSessionCreatedEvent},
	spaces::{XrPrimaryReferenceSpace, XrReferenceSpace, XrSpace},
};
use color_eyre::eyre::Result;
use glam::{Affine3A, Mat4, Vec2, Vec3};
use gluon::{Handler, Object, ObjectRef};
use openxr::{Action, ActiveActionSet, ReferenceSpaceType, SpaceLocationFlags};
use serde::{Deserialize, Serialize};
use stardust_xr_protocol::{
	field::{FieldRef as FieldRefProxy, FieldSample},
	model::{MaterialParameter, ModelHandler},
	query::{InterfaceDependency, QueryableObjectRef},
	spatial::{PartialTransform, SpatialRef as SpatialRefProxy},
	spatial_query::{
		Point, PointsQuery, PointsQueryHandle, PointsQueryHandler,
		SpatialQueryInterface as SpatialQueryInterfaceProxy,
	},
	suis::{
		Chirality, DatamapData, InputDataType, InputHandler, InputMethod, InputMethodHandler,
		SpatialData, Tip,
	},
	types::{self, Posef, Timestamp, rgba_linear},
};
use std::{
	borrow::Cow,
	collections::{HashMap, HashSet},
	fs,
	path::{Path, PathBuf},
	str::FromStr,
	sync::{Arc, OnceLock, Weak},
};
use tokio::{sync::RwLock, task::JoinHandle};
use tracing::instrument;
use zbus::Connection;

use super::CachedObject;
pub struct ControllerPlugin;
const CURSOR_MODEL_PATH: &str = "/tmp/stardust_server/models/cursor.glb";
impl Plugin for ControllerPlugin {
	fn build(&self, app: &mut App) {
		let cursor = include_bytes!("cursor.glb");
		fs::create_dir_all(
			PathBuf::from_str(CURSOR_MODEL_PATH)
				.unwrap()
				.parent()
				.unwrap(),
		);
		fs::write(CURSOR_MODEL_PATH, cursor).expect("can't write tmp cursor model file");
		app.add_systems(OxrSendActionBindings, suggest_bindings.run_if(run_once));
		app.add_systems(
			PostUpdate,
			create_spaces.run_if(on_event::<XrSessionCreatedEvent>),
		);
		app.add_systems(XrPreDestroySession, destroy_spaces);
		app.add_systems(Startup, setup.run_if(resource_exists::<OxrInstance>));
		app.add_systems(PreFrameWait, update.run_if(resource_exists::<Controllers>));
	}
}

// the api is just slightly nicer when using the bevy_mod_openxr solution okay?
fn suggest_bindings(
	instance: Res<OxrInstance>,
	actions: Res<Actions>,
	mut suggest: EventWriter<OxrSuggestActionBinding>,
	enabled_exts: Res<OxrEnabledExtensions>,
) {
	let mut bind_all = |interaction_profile: &'static str,
	                    bindings: &[(openxr::sys::Action, &[&'static str])]| {
		for (action, bindings) in bindings {
			suggest.write(OxrSuggestActionBinding {
				action: *action,
				interaction_profile: interaction_profile.into(),
				bindings: bindings.iter().copied().map(Cow::Borrowed).collect(),
			});
		}
	};
	if enabled_exts
		.other
		.iter()
		.any(|s| s == "XR_KHR_generic_controller")
	{
		bind_all(
			"/interaction_profiles/khr/generic_controller",
			&[
				(
					actions.trigger.as_raw(),
					&[
						"/user/hand/left/input/trigger/value",
						"/user/hand/right/input/trigger/value",
					],
				),
				(
					actions.stick_click.as_raw(),
					&[
						"/user/hand/left/input/thumbstick/click",
						"/user/hand/right/input/thumbstick/click",
					],
				),
				(
					actions.button.as_raw(),
					&[
						"/user/hand/left/input/primary/click",
						"/user/hand/left/input/secondary/click",
						"/user/hand/right/input/primary/click",
						"/user/hand/right/input/secondary/click",
					],
				),
				(
					actions.grip.as_raw(),
					&[
						"/user/hand/left/input/squeeze/value",
						"/user/hand/right/input/squeeze/value",
					],
				),
				(
					actions.stick.as_raw(),
					&[
						"/user/hand/left/input/thumbstick",
						"/user/hand/right/input/thumbstick",
					],
				),
				(
					actions.space.as_raw(),
					&[
						"/user/hand/left/input/aim/pose",
						"/user/hand/right/input/aim/pose",
					],
				),
			],
		);
	}
	bind_all(
		"/interaction_profiles/oculus/touch_controller",
		&[
			(
				actions.trigger.as_raw(),
				&[
					"/user/hand/left/input/trigger/value",
					"/user/hand/right/input/trigger/value",
				],
			),
			(
				actions.stick_click.as_raw(),
				&[
					"/user/hand/left/input/thumbstick/click",
					"/user/hand/right/input/thumbstick/click",
				],
			),
			(
				actions.button.as_raw(),
				&[
					"/user/hand/left/input/x/click",
					"/user/hand/left/input/y/click",
					"/user/hand/right/input/a/click",
					"/user/hand/right/input/b/click",
				],
			),
			(
				actions.grip.as_raw(),
				&[
					"/user/hand/left/input/squeeze/value",
					"/user/hand/right/input/squeeze/value",
				],
			),
			(
				actions.stick.as_raw(),
				&[
					"/user/hand/left/input/thumbstick",
					"/user/hand/right/input/thumbstick",
				],
			),
			(
				actions.space.as_raw(),
				&[
					"/user/hand/left/input/aim/pose",
					"/user/hand/right/input/aim/pose",
				],
			),
		],
	);
	bind_all(
		"/interaction_profiles/htc/vive_controller",
		&[
			(
				actions.trigger.as_raw(),
				&[
					"/user/hand/left/input/trigger/value",
					"/user/hand/right/input/trigger/value",
				],
			),
			(
				actions.stick_click.as_raw(),
				&[
					"/user/hand/left/input/trackpad/click",
					"/user/hand/right/input/trackpad/click",
				],
			),
			(
				actions.button.as_raw(),
				&[
					"/user/hand/left/input/menu/click",
					"/user/hand/right/input/menu/click",
				],
			),
			(
				actions.grip.as_raw(),
				&[
					"/user/hand/left/input/squeeze/click",
					"/user/hand/right/input/squeeze/click",
				],
			),
			(
				actions.stick.as_raw(),
				&[
					"/user/hand/left/input/trackpad",
					"/user/hand/right/input/trackpad",
				],
			),
			(
				actions.space.as_raw(),
				&[
					"/user/hand/left/input/aim/pose",
					"/user/hand/right/input/aim/pose",
				],
			),
		],
	);
	bind_all(
		"/interaction_profiles/valve/index_controller",
		&[
			(
				actions.trigger.as_raw(),
				&[
					"/user/hand/left/input/trigger/value",
					"/user/hand/right/input/trigger/value",
				],
			),
			(
				actions.stick_click.as_raw(),
				&[
					"/user/hand/left/input/thumbstick/click",
					"/user/hand/right/input/thumbstick/click",
				],
			),
			(
				actions.button.as_raw(),
				&[
					"/user/hand/left/input/a/click",
					"/user/hand/left/input/b/click",
					"/user/hand/right/input/a/click",
					"/user/hand/right/input/b/click",
				],
			),
			(
				actions.grip.as_raw(),
				&[
					"/user/hand/left/input/squeeze/value",
					"/user/hand/right/input/squeeze/value",
				],
			),
			(
				actions.stick.as_raw(),
				&[
					"/user/hand/left/input/thumbstick",
					"/user/hand/right/input/thumbstick",
				],
			),
			(
				actions.space.as_raw(),
				&[
					"/user/hand/left/input/aim/pose",
					"/user/hand/right/input/aim/pose",
				],
			),
		],
	);
	bind_all(
		"/interaction_profiles/khr/simple_controller",
		&[(
			actions.space.as_raw(),
			&[
				"/user/hand/left/input/aim/pose",
				"/user/hand/right/input/aim/pose",
			],
		)],
	);
}

fn update(
	mut controllers: ResMut<Controllers>,
	actions: Res<Actions>,
	session: Option<Res<OxrSession>>,
	ref_space: Option<Res<XrPrimaryReferenceSpace>>,
	state: Option<Res<OxrFrameState>>,
	pipelined: Option<Res<Pipelined>>,
) {
	let (Some(session), Some(state), Some(ref_space)) = (session, state, ref_space) else {
		info!("early return from main update");
		controllers.left.set_enabled(false);
		controllers.right.set_enabled(false);
		return;
	};
	debug_span!("sync actions").in_scope(|| {
		session
			.sync_actions(&[ActiveActionSet::new(&actions.set)])
			.unwrap();
	});
	let time = get_time(pipelined.is_some(), &state);
	if let Some(base_space) = controllers.base_space.as_ref() {
		let pose = session.locate_space(
			&unsafe { XrSpace::from_raw(base_space.as_raw().into_raw()) },
			&ref_space,
			time,
		);
		if let Ok(pose) = pose
			&& pose.location_flags.contains(
				SpaceLocationFlags::POSITION_TRACKED | SpaceLocationFlags::ORIENTATION_TRACKED,
			) {
			controllers
				.base_spatial
				.set_local_transform(Mat4::from_rotation_translation(
					pose.pose.orientation.to_quat(),
					pose.pose.position.to_vec3(),
				));
		}
	}
	let base_spatial = controllers.base_spatial.get_ref().clone();
	controllers
		.left
		.update(&session, &actions, time, &base_spatial);
	controllers
		.right
		.update(&session, &actions, time, &base_spatial);
}

fn create_spaces(
	session: Res<OxrSession>,
	mut controllers: ResMut<Controllers>,
	actions: Res<Actions>,
) {
	let Ok(base_space) = (**session)
		.create_reference_space(ReferenceSpaceType::LOCAL, openxr::Posef::IDENTITY)
		.inspect_err(|err| error!("failed to create openxr local space: {err}"))
		.map(Arc::new)
	else {
		return;
	};
	controllers.base_space = Some(base_space.clone());
	// if we ever need more actions than just these we should fully swith to the
	// bevy_mod_openxr provided stuff
	session.attach_action_sets(&[&actions.set]);
	session
		.sync_actions(&[ActiveActionSet::new(&actions.set)])
		.unwrap();

	let instance = session.instance();
	let left = instance.string_to_path("/user/hand/left").unwrap();
	let right = instance.string_to_path("/user/hand/right").unwrap();
	let left = actions
		.space
		.create_space((**session).clone(), left, openxr::Posef::IDENTITY)
		.unwrap();
	let right = actions
		.space
		.create_space((**session).clone(), right, openxr::Posef::IDENTITY)
		.unwrap();
	controllers.left.method = ControllerInputMethod::new(
		controllers.base_spatial.get_ref().clone(),
		base_space.clone(),
		HandSide::Left,
		left,
	)
	.ok()
	.map(|v| PION.register_object(v));
	controllers.right.method = ControllerInputMethod::new(
		controllers.base_spatial.get_ref().clone(),
		base_space.clone(),
		HandSide::Right,
		right,
	)
	.ok()
	.map(|v| PION.register_object(v));
}

fn destroy_spaces(mut controllers: ResMut<Controllers>) {
	controllers.left.method.take();
	controllers.right.method.take();
}

fn setup(instance: Res<OxrInstance>, connection: Res<DbusConnection>, mut cmds: Commands) {
	let base_spatial = SpatialObject::new(None, Mat4::IDENTITY);
	let set = instance
		.create_action_set("input_method_actions", "Input Method Action Source", 0)
		.unwrap();
	let paths = &[
		instance.string_to_path("/user/hand/left").unwrap(),
		instance.string_to_path("/user/hand/right").unwrap(),
	];
	let actions = Actions {
		trigger: set.create_action("trigger", "Select", paths).unwrap(),
		stick_click: set.create_action("stick_click", "Middle", paths).unwrap(),
		button: set.create_action("face_button", "Context", paths).unwrap(),
		grip: set.create_action("grip", "Grab", paths).unwrap(),
		stick: set.create_action("stick", "Scroll", paths).unwrap(),
		space: set.create_action("pose", "Location", paths).unwrap(),
		set,
	};
	let controllers = Controllers {
		left: OxrControllerInput::new(HandSide::Left, base_spatial.get_ref()).unwrap(),
		right: OxrControllerInput::new(HandSide::Right, base_spatial.get_ref()).unwrap(),
		base_space: None,
		base_spatial,
	};
	cmds.insert_resource(controllers);
	cmds.insert_resource(actions);
}

#[derive(Default, Debug, Deserialize, Serialize)]
struct ControllerDatamap {
	select: f32,
	middle: f32,
	context: f32,
	grab: f32,
	scroll: Vec2,
}
#[derive(Resource)]
struct Actions {
	set: openxr::ActionSet,
	trigger: openxr::Action<f32>,
	stick_click: openxr::Action<f32>,
	button: openxr::Action<f32>,
	grip: openxr::Action<f32>,
	space: openxr::Action<openxr::Posef>,
	stick: openxr::Action<openxr::Vector2f>,
}
#[derive(Resource)]
struct Controllers {
	left: OxrControllerInput,
	right: OxrControllerInput,
	base_space: Option<Arc<openxr::Space>>,
	base_spatial: gluon::ObjectRef<SpatialObject>,
}

pub struct OxrControllerInput {
	aim_spatial: gluon::ObjectRef<SpatialObject>,
	side: HandSide,
	model: OnceLock<ObjectRef<Model>>,
	model_part: OnceLock<ObjectRef<ModelPart>>,
	model_task: Option<JoinHandle<(ObjectRef<Model>, ObjectRef<ModelPart>)>>,
	method: Option<Object<ControllerInputMethod>>,
	was_enabled: bool,
	captured: bool,
}
impl OxrControllerInput {
	fn new(side: HandSide, base_space: &ObjectRef<SpatialRef>) -> Result<Self> {
		let aim_spatial = SpatialObject::new(Some(&***base_space), Mat4::from_scale(Vec3::ZERO));
		let model_spatial =
			SpatialObject::new(Some(&aim_spatial), Mat4::from_scale(Vec3::splat(0.02)));
		let model_task = tokio::spawn(async move {
			let model = Model::new(
				model_spatial,
				types::Resource::Direct {
					path: CURSOR_MODEL_PATH.into(),
				},
				// using direct path, no prefixes required
				Arc::new(Vec::new()),
			)
			.await
			.unwrap();
			let model_part = model
				.get_part(
					// unused by impl, and 0 anyway
					gluon::Context {
						sender_pid: 0,
						sender_euid: 0,
					},
					"Cursor".to_string(),
				)
				.await
				.unwrap()
				.owned()
				.unwrap();
			(model, model_part)
		});
		Ok(OxrControllerInput {
			side,
			model: OnceLock::new(),
			model_part: OnceLock::new(),
			was_enabled: false,
			aim_spatial,
			model_task: Some(model_task),
			method: None,
			captured: false,
		})
	}
	pub fn set_enabled(&self, enabled: bool) {
		self.aim_spatial.set_local_transform_components(
			None,
			PartialTransform::from_scale(Vec3::splat(enabled as u8 as f32)),
		);
	}
	fn update(
		&mut self,
		session: &OxrSession,
		actions: &Actions,
		time: openxr::Time,
		base_space: &ObjectRef<SpatialRef>,
	) {
		if self.model_task.as_ref().is_some_and(|v| v.is_finished()) {
			let (model, part) = now_or_never(self.model_task.take().unwrap())
				.unwrap()
				.unwrap();
			self.model.set(model);
			self.model_part.set(part);
		}
		let Some(method) = self.method.as_ref() else {
			return;
		};
		let _span = debug_span!("locate space").entered();
		let pose = method.locate_pose(base_space, time);
		let enabled = pose.is_some();
		drop(_span);
		self.set_enabled(enabled);
		if let Some(pose) = pose {
			let world_transform = Mat4::from(Affine3A::from_rotation_translation(
				pose.orientation.into(),
				pose.position.into(),
			));
			if let Some(part) = self.model_part.get() {
				part.set_material_parameter(
					"roughness".to_string(),
					MaterialParameter::Float { value: 1.0 },
				);
				part.set_material_parameter(
					"color".to_string(),
					MaterialParameter::Color {
						value: if method.sender.active_capture.blocking_read().is_some() {
							rgba_linear!(0.0, 1.0, 0.75, 1.0)
						} else {
							rgba_linear!(1.0, 1.0, 1.0, 1.0)
						},
					},
				);
			}
			if let Some(handle) = method.query_handle.get() {
				handle.update_points([Point {
					point: pose.position,
					margin: 0.5,
				}]);
			}
			method.pose.blocking_write().replace(pose);
			self.aim_spatial.set_local_transform(world_transform);
		}
		let path = method
			.space
			.instance()
			.string_to_path(match self.side {
				HandSide::Left => "/user/hand/left",
				HandSide::Right => "/user/hand/right",
			})
			.unwrap();
		if let Ok(path) = session.current_interaction_profile(path)
			&& path != openxr::Path::NULL
			&& let Ok(path) = session.instance().path_to_string(path)
			&& path == "/interaction_profiles/khr/simple_controller"
		{
			self.set_enabled(false);
		}

		fn get<T: openxr::ActionInput + Default>(
			session: &OxrSession,
			path: openxr::Path,
			action: &Action<T>,
		) -> T {
			action
				.state(session, path)
				.map(|v| v.current_state)
				.unwrap_or_default()
		}
		let _span = debug_span!("apply datamap").entered();
		*method.datamap.blocking_write() = ControllerDatamap {
			select: get(session, path, &actions.trigger),
			middle: get(session, path, &actions.stick_click) as u32 as f32,
			context: get(session, path, &actions.button) as u32 as f32,
			grab: get(session, path, &actions.grip),
			scroll: get(session, path, &actions.stick).to_vec2(),
		};
		drop(_span);

		let input_method = InputMethod::from_handler(method);
		let ts = session
			.instance()
			.xr_to_timestamp(time)
			.unwrap_or_else(Timestamp::now);
		method.sender.clone().send(&***method, input_method, ts);
	}
}
#[derive(Debug, Handler)]
struct ControllerInputMethod {
	side: HandSide,
	base_space: DebugWrapper<Arc<openxr::Space>>,
	base_spatial: gluon::ObjectRef<SpatialRef>,
	space: DebugWrapper<openxr::Space>,
	_query: gluon::Object<PointsQueryCache>,
	sender: Arc<InputSender<FieldSample>>,
	pose: RwLock<Option<Posef>>,
	datamap: RwLock<ControllerDatamap>,
	query_handle: Arc<OnceLock<PointsQueryHandle>>,
}
impl ControllerInputMethod {
	fn new(
		base_spatial: gluon::ObjectRef<SpatialRef>,
		base_space: Arc<openxr::Space>,
		side: HandSide,
		space: openxr::Space,
	) -> Result<Self, gluon::SendError> {
		let (query_cache, objects_arc, capture_requests) = QueryCache::new();
		let sender = Arc::new(InputSender::new(objects_arc, capture_requests));

		let query = PION.register_object(PointsQueryCache(query_cache));
		let proxy = PointsQueryHandler::from_handler(&query);
		let query_handle = Arc::new(OnceLock::new());
		let base_spatial_ref = SpatialRefProxy::from_handler(&base_spatial);
		tokio::spawn({
			let query_handle = query_handle.clone();
			async move {
				let spatial_query_interface = SpatialQueryInterface::new(&Arc::default());
				let spatial_query_interface_proxy =
					SpatialQueryInterfaceProxy::from_handler(&spatial_query_interface);
				let handle = spatial_query_interface_proxy
					.points_query(PointsQuery {
						handler: proxy,
						interfaces: vec![InterfaceDependency {
							id: InputHandler::QUERY_INTERFACE.to_string(),
							optional: false,
						}],
						points: vec![],
						reference_spatial: base_spatial_ref,
					})
					.await
					.inspect_err(|err| error!("failed to create query: {err}"));
				if let Ok(Ok(handle)) = handle {
					info!("setting point");
					query_handle.set(handle);
				}
			}
		});

		Ok(Self {
			side,
			base_space: base_space.into(),
			base_spatial,
			space: space.into(),
			_query: query,
			sender,
			pose: RwLock::new(None),
			datamap: RwLock::new(ControllerDatamap::default()),
			query_handle,
		})
	}
	fn locate_pose(
		&self,
		relative_to: &ObjectRef<SpatialRef>,
		time: openxr::Time,
	) -> Option<Posef> {
		let pose = self
			.space
			.locate(&self.base_space, time)
			.inspect_err(|err| error!("Error while locating controller pose: {err}"))
			.ok()?;
		let valid = pose.location_flags.contains(
			SpaceLocationFlags::POSITION_VALID
				| SpaceLocationFlags::POSITION_TRACKED
				| SpaceLocationFlags::ORIENTATION_VALID
				| SpaceLocationFlags::ORIENTATION_TRACKED,
		);
		valid.then(|| {
			let mat = Spatial::space_to_space_matrix(Some(&self.base_spatial), Some(relative_to));
			Posef {
				position: mat.transform_point3(pose.pose.position.to_vec3()).into(),
				orientation: (mat.to_scale_rotation_translation().1
					* pose.pose.orientation.to_quat())
				.into(),
			}
		})
	}
	fn localize_pose(&self, relative_to: &Spatial, pose: Posef) -> Posef {
		let mat = Spatial::space_to_space_matrix(Some(&self.base_spatial), Some(relative_to));
		Posef {
			position: mat.transform_point3(pose.position.into()).into(),
			orientation: (mat.to_scale_rotation_translation().1 * Quat::from(pose.orientation))
				.into(),
		}
	}
	fn pose_distance(field: &Field, space: &Spatial, pose: Posef) -> f32 {
		field.sample(space, pose.position.into()).distance
	}
}
impl InputSource for ControllerInputMethod {
	type QueryValue = FieldSample;

	fn order_handlers_and_captures(
		&self,
		objects: &HashMap<QueryableObjectRef, CachedObject<Self::QueryValue>>,
		capture_requests: &HashSet<InputHandler>,
	) -> (Vec<InputHandler>, Option<InputHandler>) {
		let pose = *self.pose.blocking_read();
		let Some(pose) = pose else {
			self.sender.active_capture.blocking_write().take();
			return (vec![], None);
		};
		let current_capture = self.sender.active_capture.blocking_read().clone();
		let capture = if let Some(cap) = current_capture {
			if objects.values().any(|e| e.handler == cap) {
				Some(cap)
			} else {
				self.sender.active_capture.blocking_write().take();
				None
			}
		} else {
			let mut order: Vec<_> = objects
				.values()
				.filter(|e| e.spatial.is_some() && capture_requests.contains(&e.handler))
				.map(|e| {
					let dist = Self::pose_distance(&e.field.data, &self.base_spatial, pose);
					(dist, e.handler.clone())
				})
				.collect();
			order.sort_by(|(d1, _), (d2, _)| d1.total_cmp(d2));
			let promoted = order.first().map(|(_, v)| v.clone());
			if let Some(ref p) = promoted {
				*self.sender.active_capture.blocking_write() = Some(p.clone());
			}
			promoted
		};

		if let Some(ref cap) = capture {
			let handlers: Vec<_> = objects
				.values()
				.filter(|e| e.spatial.is_some() && &e.handler == cap)
				.map(|e| e.handler.clone())
				.collect();
			return (handlers, capture);
		}

		let mut order: Vec<_> = objects
			.values()
			.filter(|e| e.spatial.is_some())
			.map(|e| {
				let dist = Self::pose_distance(&e.field.data, &self.base_spatial, pose);
				(dist, e.handler.clone())
			})
			.collect();
		order.sort_by(|(d1, _), (d2, _)| d1.total_cmp(d2));
		(order.into_iter().map(|(_, h)| h).collect(), None)
	}

	fn spatial_data(&self, handler_spatial: &SpatialRef, handler_field: &Field) -> SpatialData {
		let pose = self.pose.blocking_read().unwrap();
		let pose = self.localize_pose(handler_spatial, pose);
		SpatialData {
			input: InputDataType::Tip {
				data: Tip {
					pose,
					chirality: Some(match self.side {
						HandSide::Left => Chirality::Left,
						HandSide::Right => Chirality::Right,
					}),
					grip_pose: None,
					grip_surface_pose: None,
					simulated_hand: None,
				},
			},
			distance: Self::pose_distance(handler_field, handler_spatial, pose),
		}
	}

	fn datamap(&self) -> HashMap<String, stardust_xr_protocol::suis::DatamapData> {
		let ControllerDatamap {
			select,
			middle,
			context,
			grab,
			scroll,
		} = *self.datamap.blocking_read();
		HashMap::from(
			[
				("select", DatamapData::Float { value: select }),
				("middle", DatamapData::Float { value: middle }),
				("context", DatamapData::Float { value: context }),
				("grab", DatamapData::Float { value: grab }),
				(
					"scroll",
					DatamapData::Vec2 {
						value: scroll.into(),
					},
				),
			]
			.map(|(k, v)| (k.to_string(), v)),
		)
	}
}
impl InputMethodHandler for ControllerInputMethod {
	fn request_capture(
		&self,
		_ctx: gluon::Context,
		handler: stardust_xr_protocol::suis::InputHandler,
	) -> impl Future<Output = Option<stardust_xr_protocol::suis::InputMethodCapture>> {
		self.sender.grant_capture(handler)
	}

	async fn get_spatial_data(
		&self,
		_ctx: gluon::Context,
		handler: stardust_xr_protocol::suis::InputHandler,
		time: Timestamp,
	) -> Option<SpatialData> {
		let spatial = handler.get_spatial().await.ok()?.owned()?;
		let field = handler.get_field().await.ok()?.owned()?;
		let time = self.base_space.instance().timestamp_to_xr(time)?;
		let pose = self.locate_pose(&spatial, time)?;
		Some(SpatialData {
			input: InputDataType::Tip {
				data: Tip {
					pose,
					chirality: Some(match self.side {
						HandSide::Left => Chirality::Left,
						HandSide::Right => Chirality::Right,
					}),
					grip_pose: None,
					grip_surface_pose: None,
					simulated_hand: None,
				},
			},
			distance: Self::pose_distance(&field.data, &spatial, pose),
		})
	}
}

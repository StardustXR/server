use crate::{
	DbusConnection, PreFrameWait, get_time,
	nodes::{
		ProxyExt,
		drawable::model::{Model, ModelPart},
		fields::Field,
		spatial::{Spatial, SpatialObject, SpatialRef},
	},
	objects::{
		DebugWrapper, Tracked,
		input::{
			CachedHandler, DatamapBuilder, FrameDriver, InputMethod as InputMethodNode,
			InputMethodHelper, order_by_distance, spawn_frame_driver,
		},
	},
	openxr_helpers::ConvertTimespec,
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
use gluon_ipc::{Handler, LocalRef, Node, RefExt};
use openxr::{Action, ActiveActionSet, ReferenceSpaceType, SpaceLocationFlags, sys::Handle as _};
use serde::{Deserialize, Serialize};
use stardust_xr_protocol::{
	field::FieldSample,
	model::{MaterialParameter, ModelHandler, ModelLocal, ModelPartLocal},
	query::QueryableId,
	spatial::{PartialTransform, Spatial as SpatialProxy, SpatialRef as SpatialRefProxy},
	spatial_query::{Point, PointsQueryHandle},
	suis::{Chirality, DatamapData, InputDataType, InputHandler, Tip},
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
	if enabled_exts.khr_generic_controller {
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
	// core since 1.1, so no XR_FB_touch_controller_pro needed
	let touch_pro = "/interaction_profiles/meta/touch_pro_controller";
	for profile in [touch_pro, "/interaction_profiles/oculus/touch_controller"] {
		bind_all(
			profile,
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
	}
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
	let profiles = [
		"/interaction_profiles/oculus/touch_controller",
		"/interaction_profiles/htc/vive_controller",
		"/interaction_profiles/valve/index_controller",
		"/interaction_profiles/khr/simple_controller",
	];
	for profile in enabled_exts
		.khr_generic_controller
		.then_some("/interaction_profiles/khr/generic_controller")
		.into_iter()
		.chain([touch_pro])
		.chain(profiles)
	{
		bind_all(
			profile,
			&[(
				actions.grip_pose.as_raw(),
				&[
					"/user/hand/left/input/grip/pose",
					"/user/hand/right/input/grip/pose",
				],
			)],
		);
	}
	// palm_ext isn't defined for generic_controller, and one bad path rejects the whole profile
	if enabled_exts.ext_palm_pose {
		for profile in profiles {
			bind_all(
				profile,
				&[(
					actions.palm_pose.as_raw(),
					&[
						"/user/hand/left/input/palm_ext/pose",
						"/user/hand/right/input/palm_ext/pose",
					],
				)],
			);
		}
	}
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
	enabled_exts: Res<OxrEnabledExtensions>,
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
	let space = |action: &Action<openxr::Posef>, path| {
		action
			.create_space(&*session, path, openxr::Posef::IDENTITY)
			.unwrap()
	};
	let base_spatial = controllers.base_spatial.get_ref().clone();
	for (side, path) in [(HandSide::Left, left), (HandSide::Right, right)] {
		let helper = ControllerInputMethod::new(
			base_spatial.clone(),
			base_space.clone(),
			side,
			space(&actions.space, path),
			space(&actions.grip_pose, path),
			enabled_exts
				.ext_palm_pose
				.then(|| space(&actions.palm_pose, path)),
		);
		let Ok((method, _, query_handle)) = InputMethodNode::new_points(
			helper,
			(**base_spatial).clone(),
			base_spatial.proxy().clone(),
			vec![],
		)
		.inspect_err(|err| error!("failed to create {side:?} controller input method: {err}")) else {
			continue;
		};
		let controller = match side {
			HandSide::Left => &mut controllers.left,
			HandSide::Right => &mut controllers.right,
		};
		for pose in controller.poses() {
			pose.tracked.get_mut_data_blocking().method = Arc::downgrade(method.handler());
		}
		controller.driver = Some(spawn_frame_driver(&method));
		controller.query_handle = Some(query_handle);
		controller.method = Some(method);
	}
}

fn destroy_spaces(mut controllers: ResMut<Controllers>) {
	controllers.left.method.take();
	controllers.right.method.take();
	controllers.left.driver.take();
	controllers.right.driver.take();
	controllers.left.query_handle.take();
	controllers.right.query_handle.take();
	for pose in controllers.left.poses().chain(controllers.right.poses()) {
		pose.tracked.get_mut_data_blocking().method = Weak::new();
	}
}

fn setup(
	instance: Res<OxrInstance>,
	enabled_exts: Res<OxrEnabledExtensions>,
	connection: Res<DbusConnection>,
	mut cmds: Commands,
) {
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
		grip_pose: set.create_action("grip_pose", "Grip", paths).unwrap(),
		palm_pose: set.create_action("palm_pose", "Palm", paths).unwrap(),
		set,
	};
	let controllers = Controllers {
		left: OxrControllerInput::new(
			HandSide::Left,
			base_spatial.get_ref(),
			enabled_exts.ext_palm_pose,
		)
		.unwrap(),
		right: OxrControllerInput::new(
			HandSide::Right,
			base_spatial.get_ref(),
			enabled_exts.ext_palm_pose,
		)
		.unwrap(),
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
	grip_pose: openxr::Action<openxr::Posef>,
	palm_pose: openxr::Action<openxr::Posef>,
	stick: openxr::Action<openxr::Vector2f>,
}
#[derive(Resource)]
struct Controllers {
	left: OxrControllerInput,
	right: OxrControllerInput,
	base_space: Option<Arc<openxr::Space>>,
	base_spatial: gluon_ipc::LocalRef<SpatialProxy, SpatialObject>,
}

#[derive(Debug)]
struct OxrControllerInputTrackedState {
	method: Weak<InputMethodNode<ControllerInputMethod>>,
	space: fn(&ControllerInputMethod) -> Option<&openxr::Space>,
	spatial: LocalRef<SpatialProxy, SpatialObject>,
}
impl OxrControllerInputTrackedState {
	fn get_pose(&self, relative_to: &Spatial, at: Timestamp) -> (Option<types::Posef>, bool) {
		if let Some(method) = self.method.upgrade() {
			let Some(time) = method.base_space.instance().timestamp_to_xr(at) else {
				return (None, false);
			};
			let Some(pose) = (self.space)(&method)
				.and_then(|space| method.locate_pose(space, relative_to, time))
			else {
				return (None, false);
			};
			(Some(pose), true)
		} else {
			let mat = crate::nodes::spatial::Spatial::space_to_space_matrix(
				Some(&self.spatial),
				Some(relative_to),
			);
			let (_, rot, pos) = mat.to_scale_rotation_translation();
			(
				Some(stardust_xr_protocol::types::Posef {
					position: pos.into(),
					orientation: rot.into(),
				}),
				true,
			)
		}
	}
}
struct TrackedPose {
	spatial: LocalRef<SpatialProxy, SpatialObject>,
	tracked: Tracked<OxrControllerInputTrackedState>,
}
impl TrackedPose {
	fn new(
		base_space: &LocalRef<SpatialRefProxy, SpatialRef>,
		pion_path: &str,
		space: fn(&ControllerInputMethod) -> Option<&openxr::Space>,
	) -> Self {
		let spatial = SpatialObject::new(Some(&***base_space), Mat4::from_scale(Vec3::ZERO));
		let tracked = Tracked::new(
			spatial.get_ref().proxy().clone(),
			OxrControllerInputTrackedState::get_pose,
			false,
			pion_path,
			OxrControllerInputTrackedState {
				method: Weak::new(),
				space,
				spatial: spatial.clone(),
			},
		)
		.unwrap();
		TrackedPose { spatial, tracked }
	}
	fn set_enabled(&self, enabled: bool) {
		self.tracked.tracked_blocking(enabled);
		self.spatial.set_local_transform_components(
			None,
			PartialTransform::from_scale(Vec3::splat(enabled as u8 as f32)),
		);
	}
	fn set_pose(&self, pose: Option<Posef>) {
		self.set_enabled(pose.is_some());
		if let Some(pose) = pose {
			self.spatial
				.set_local_transform(Mat4::from(Affine3A::from_rotation_translation(
					pose.orientation.into(),
					pose.position.into(),
				)));
		}
	}
}
pub struct OxrControllerInput {
	aim: TrackedPose,
	grip: TrackedPose,
	palm: Option<TrackedPose>,
	side: HandSide,
	model: OnceLock<ModelLocal<Model>>,
	model_part: OnceLock<Arc<ModelPart>>,
	model_task: Option<JoinHandle<(ModelLocal<Model>, Arc<ModelPart>)>>,
	method: Option<gluon_ipc::Node<InputMethodNode<ControllerInputMethod>>>,
	driver: Option<FrameDriver>,
	query_handle: Option<Arc<OnceLock<PointsQueryHandle>>>,
	was_enabled: bool,
	captured: bool,
}
impl OxrControllerInput {
	fn new(
		side: HandSide,
		base_space: &LocalRef<SpatialRefProxy, SpatialRef>,
		palm: bool,
	) -> Result<Self> {
		let pion_path = match side {
			HandSide::Left => "stardust-controller/left",
			HandSide::Right => "stardust-controller/right",
		};
		let aim = TrackedPose::new(base_space, pion_path, |m| Some(&m.space));
		let grip = TrackedPose::new(base_space, &format!("{pion_path}/grip"), |m| Some(&m.grip));
		let palm = palm.then(|| {
			TrackedPose::new(base_space, &format!("{pion_path}/palm"), |m| {
				m.palm.as_deref()
			})
		});
		let model_spatial =
			SpatialObject::new(Some(&aim.spatial), Mat4::from_scale(Vec3::splat(0.02)));
		let model_task = tokio::spawn(async move {
			let model = Model::new(
				model_spatial.handler().clone(),
				types::Resource::Direct {
					path: CURSOR_MODEL_PATH.into(),
				},
				// using direct path, no prefixes required
				Arc::new(Vec::new()),
			)
			.await
			.unwrap();
			let model_part = model
				.proxy()
				.get_part("Cursor".to_string())
				.await
				.unwrap()
				.unwrap()
				.local_handler::<ModelPart>()
				.unwrap();
			(model, model_part)
		});
		Ok(OxrControllerInput {
			side,
			model: OnceLock::new(),
			model_part: OnceLock::new(),
			was_enabled: false,
			aim,
			grip,
			palm,
			model_task: Some(model_task),
			method: None,
			driver: None,
			query_handle: None,
			captured: false,
		})
	}
	fn poses(&self) -> impl Iterator<Item = &TrackedPose> {
		[Some(&self.aim), Some(&self.grip), self.palm.as_ref()]
			.into_iter()
			.flatten()
	}
	pub fn set_enabled(&self, enabled: bool) {
		for pose in self.poses() {
			pose.set_enabled(enabled);
		}
	}
	fn update(
		&mut self,
		session: &OxrSession,
		actions: &Actions,
		time: openxr::Time,
		base_space: &Arc<SpatialRef>,
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
		let aim_pose = method.locate_pose(&method.space, base_space, time);
		self.aim.set_pose(aim_pose);
		self.grip
			.set_pose(method.locate_pose(&method.grip, base_space, time));
		if let (Some(palm), Some(space)) = (&self.palm, &method.palm) {
			palm.set_pose(method.locate_pose(space, base_space, time));
		}
		drop(_span);
		let ts = session
			.instance()
			.xr_to_timestamp(time)
			.unwrap_or_else(Timestamp::now);
		*method.pose.blocking_write() = aim_pose.map(|pose| (ts, pose));
		if let Some(pose) = aim_pose {
			if let Some(part) = self.model_part.get() {
				part.set_material_parameter(
					"roughness".to_string(),
					MaterialParameter::Float { value: 1.0 },
				);
				part.set_material_parameter(
					"color".to_string(),
					MaterialParameter::Color {
						value: if method.active_capture_blocking().is_some() {
							rgba_linear!(0.0, 1.0, 0.75, 1.0)
						} else {
							rgba_linear!(1.0, 1.0, 1.0, 1.0)
						},
					},
				);
			}
			if let Some(handle) = self.query_handle.as_ref().and_then(|h| h.get()) {
				_ = handle.update(vec![Point {
					point: pose.position,
					margin: 0.5,
				}]);
			}
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

		if let Some(driver) = self.driver.as_ref() {
			driver.frame(ts);
		}
	}
}
#[derive(Debug)]
struct ControllerInputMethod {
	side: HandSide,
	base_space: DebugWrapper<Arc<openxr::Space>>,
	base_spatial: gluon_ipc::LocalRef<SpatialRefProxy, SpatialRef>,
	space: DebugWrapper<openxr::Space>,
	grip: DebugWrapper<openxr::Space>,
	palm: Option<DebugWrapper<openxr::Space>>,
	/// the frame's pose and the moment it was located for, so a pull for that same moment
	/// doesn't relocate
	pose: RwLock<Option<(Timestamp, Posef)>>,
	datamap: RwLock<ControllerDatamap>,
}
impl ControllerInputMethod {
	fn new(
		base_spatial: gluon_ipc::LocalRef<SpatialRefProxy, SpatialRef>,
		base_space: Arc<openxr::Space>,
		side: HandSide,
		space: openxr::Space,
		grip: openxr::Space,
		palm: Option<openxr::Space>,
	) -> Self {
		Self {
			side,
			base_space: base_space.into(),
			base_spatial,
			space: space.into(),
			grip: grip.into(),
			palm: palm.map(Into::into),
			pose: RwLock::new(None),
			datamap: RwLock::new(ControllerDatamap::default()),
		}
	}

	async fn pose_at(&self, time: Timestamp) -> Option<Posef> {
		match *self.pose.read().await {
			Some((at, pose)) if at == time => Some(pose),
			_ => self.locate_pose(
				&self.space,
				&self.base_spatial,
				self.base_space.instance().timestamp_to_xr(time)?,
			),
		}
	}

	fn locate_pose(
		&self,
		space: &openxr::Space,
		relative_to: &Spatial,
		time: openxr::Time,
	) -> Option<Posef> {
		let pose = space
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
	fn pose_distance(field: &Field, space: &Spatial, pose: Posef) -> f32 {
		field.sample(space, pose.position.into()).distance
	}
}
impl InputMethodHelper for ControllerInputMethod {
	type QueryValue = FieldSample;

	async fn order_handlers_and_captures(
		&self,
		handlers: &HashMap<QueryableId, CachedHandler<FieldSample>>,
		capture_requests: &HashSet<InputHandler>,
		active_capture: Option<&InputHandler>,
	) -> (Vec<InputHandler>, Option<InputHandler>) {
		let Some((_, pose)) = *self.pose.read().await else {
			return (
				active_capture.cloned().into_iter().collect(),
				active_capture.cloned(),
			);
		};
		order_by_distance(handlers, capture_requests, active_capture, |e| {
			Some(Self::pose_distance(&e.field, &self.base_spatial, pose).abs())
		})
	}

	async fn input_data(&self, time: Timestamp) -> Option<InputDataType> {
		let t = self.base_space.instance().timestamp_to_xr(time);
		Some(InputDataType::Tip {
			data: Tip {
				pose: self.pose_at(time).await?,
				chirality: Some(match self.side {
					HandSide::Left => Chirality::Left,
					HandSide::Right => Chirality::Right,
				}),
				grip_pose: t.and_then(|t| self.locate_pose(&self.grip, &self.base_spatial, t)),
				// palm_ext is what 1.1 promoted to grip_surface
				grip_surface_pose: t
					.zip(self.palm.as_deref())
					.and_then(|(t, palm)| self.locate_pose(palm, &self.base_spatial, t)),
				simulated_hand: None,
			},
		})
	}

	async fn datamap(&self) -> HashMap<String, DatamapData> {
		let ControllerDatamap {
			select,
			middle,
			context,
			grab,
			scroll,
		} = *self.datamap.read().await;
		DatamapBuilder::default()
			.f32("select", select)
			.f32("middle", middle)
			.f32("context", context)
			.f32("grab", grab)
			.vec2("scroll", scroll)
			.build()
	}
}

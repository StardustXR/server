use super::{CaptureManager, get_sorted_handlers};
use crate::{
	DbusConnection, PreFrameWait,
	core::client::INTERNAL_CLIENT,
	get_time,
	nodes::{
		Node, OwnedNode,
		drawable::{
			MaterialParameter,
			model::{Model, ModelPart},
		},
		fields::{Field, FieldTrait},
		input::{INPUT_HANDLER_REGISTRY, InputDataType, InputHandler, InputMethod, Tip},
		spatial::Spatial,
	},
	objects::{AsyncTracked, ObjectHandle, SpatialRef, Tracked},
};
use bevy::{asset::Handle, ecs::resource::Resource};
use bevy::{math::Affine3, prelude::*};
use bevy_mod_openxr::{
	action_binding::{OxrSendActionBindings, OxrSuggestActionBinding},
	exts::OxrEnabledExtensions,
	helper_traits::{ToIsometry3d, ToVec2},
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
use openxr::{Action, ActiveActionSet, SpaceLocationFlags};
use serde::{Deserialize, Serialize};
use stardust_xr_wire::values::{Datamap, ResourceID, color::Rgb};
use std::{
	borrow::Cow,
	fs,
	path::{Path, PathBuf},
	str::FromStr,
	sync::Arc,
};
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
	controllers
		.left
		.update(&session, &actions, time, ref_space.0);
	controllers
		.right
		.update(&session, &actions, time, ref_space.0);
}

fn create_spaces(
	session: Res<OxrSession>,
	mut controllers: ResMut<Controllers>,
	actions: Res<Actions>,
) {
	// if we ever need more actions than just these we should fully swith to the
	// bevy_mod_openxr provided stuff
	session.attach_action_sets(&[&actions.set]);
	session
		.sync_actions(&[ActiveActionSet::new(&actions.set)])
		.unwrap();

	let instance = session.instance();
	let left = instance.string_to_path("/user/hand/left").unwrap();
	let right = instance.string_to_path("/user/hand/right").unwrap();
	let left = session
		.create_action_space(&actions.space, left, Isometry3d::IDENTITY)
		.unwrap();
	let right = session
		.create_action_space(&actions.space, right, Isometry3d::IDENTITY)
		.unwrap();
	controllers.left.space = Some(left);
	controllers.right.space = Some(right);
}

fn destroy_spaces(session: Res<OxrSession>, mut controllers: ResMut<Controllers>) {
	if let Some(space) = controllers.left.space.take() {
		session.destroy_space(space);
	}
	if let Some(space) = controllers.right.space.take() {
		session.destroy_space(space);
	}
}

fn setup(instance: Res<OxrInstance>, connection: Res<DbusConnection>, mut cmds: Commands) {
	tokio::task::spawn({
		let connection = connection.clone();
		async move {
			connection
				.request_name("org.stardustxr.Controllers")
				.await
				.unwrap();
		}
	});
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
		left: OxrControllerInput::new(&connection, HandSide::Left).unwrap(),
		right: OxrControllerInput::new(&connection, HandSide::Right).unwrap(),
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
}

pub struct OxrControllerInput {
	object_handle: ObjectHandle<SpatialRef>,
	input: Arc<InputMethod>,
	side: HandSide,
	model: Arc<Model>,
	model_part: Arc<ModelPart>,
	capture_manager: CaptureManager,
	datamap: ControllerDatamap,
	tracked: AsyncTracked,
	space: Option<XrSpace>,
	_model_node: OwnedNode,
}
impl OxrControllerInput {
	fn new(connection: &Connection, side: HandSide) -> Result<Self> {
		let path = "/org/stardustxr/Controller/".to_string()
			+ match side {
				HandSide::Left => "left",
				HandSide::Right => "right",
			};
		let (spatial, object_handle) = SpatialRef::create(connection, &path);
		let tracked = AsyncTracked::new(connection, &path);
		let tip = InputDataType::Tip(Tip::default());
		let node = spatial.node().unwrap();
		node.set_enabled(false);
		let model_node = Arc::new(Node::generate(&INTERNAL_CLIENT, true));
		let model_spatial = Spatial::add_to(
			&model_node,
			Some(spatial.clone()),
			Mat4::from_scale(Vec3::splat(0.02)),
		);
		let model =
			Model::add_to(&model_node, ResourceID::Direct(CURSOR_MODEL_PATH.into())).unwrap();
		let model_part = model.get_model_part("Cursor".to_string()).unwrap();
		let input = InputMethod::add_to(
			&node,
			tip,
			Datamap::from_typed(ControllerDatamap::default())?,
		)?;
		Ok(OxrControllerInput {
			object_handle,
			input,
			side,
			model,
			model_part,
			capture_manager: CaptureManager::default(),
			datamap: Default::default(),
			tracked,
			space: None,
			_model_node: OwnedNode(model_node),
		})
	}
	#[instrument(level = "debug", skip(self))]
	pub fn set_enabled(&self, enabled: bool) {
		if let Some(node) = self.input.spatial.node() {
			node.set_enabled(enabled);
		}
		self.tracked.set_tracked(enabled);
	}
	fn update(
		&mut self,
		session: &OxrSession,
		actions: &Actions,
		time: openxr::Time,
		ref_space: XrReferenceSpace,
	) {
		let Some(space) = self.space.as_ref() else {
			return;
		};
		let _span = debug_span!("locate space").entered();
		let Ok(location) = session
			.locate_space(space, &ref_space, time)
			.inspect_err(|err| error!("error while locating controller space: {err}"))
		else {
			return;
		};
		let enabled = location.location_flags.contains(
			SpaceLocationFlags::POSITION_VALID
				| SpaceLocationFlags::POSITION_TRACKED
				| SpaceLocationFlags::ORIENTATION_VALID
				| SpaceLocationFlags::ORIENTATION_TRACKED,
		);
		drop(_span);
		self.set_enabled(enabled);
		if enabled {
			let world_transform = Mat4::from(Affine3A::from(location.pose.to_xr_pose()));
			self.model_part
				.set_material_parameter("roughness".to_string(), MaterialParameter::Float(1.0));
			self.model_part.set_material_parameter(
				"color".to_string(),
				MaterialParameter::Color(stardust_xr_wire::values::Color::new(
					if self.capture_manager.capture.upgrade().is_none() {
						Rgb::new(1.0, 1.0, 1.0)
					} else {
						Rgb::new(0.0, 1.0, 0.75)
					},
					1.0,
				)),
			);
			self.input.spatial.set_local_transform(world_transform);
		}
		let path = session
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
		self.datamap = ControllerDatamap {
			select: get(session, path, &actions.trigger),
			middle: get(session, path, &actions.stick_click) as u32 as f32,
			context: get(session, path, &actions.button) as u32 as f32,
			grab: get(session, path, &actions.grip),
			scroll: get(session, path, &actions.stick).to_vec2(),
		};
		let input = self.input.data().clone();

		*self.input.datamap.lock() = Datamap::from_typed(&self.datamap).unwrap();
		drop(_span);

		let distance_calculator = |space: &Arc<Spatial>, _data: &InputDataType, field: &Field| {
			Some(field.distance(space, [0.0; 3].into()).abs())
		};

		if self
			.capture_manager
			.update(&self.input, distance_calculator)
		{
			return;
		}

		let sorted_handlers = get_sorted_handlers(&self.input, distance_calculator);
		let order: Vec<Arc<InputHandler>> = sorted_handlers
			.into_iter()
			.map(|(handler, _)| handler)
			.collect();
		self.input.set_handler_capture_order(order, vec![]);
	}
}

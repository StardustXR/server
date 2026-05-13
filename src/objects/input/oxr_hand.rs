use super::{CachedObject, InputSender, InputSource, PointsQueryCache, QueryCache};
use crate::nodes::ProxyExt;
use crate::nodes::drawable::model::HoldoutExtension;
use crate::nodes::fields::Field;
use crate::nodes::spatial::{Spatial, SpatialObject, SpatialRef};
use crate::openxr_helpers::ConvertTimespec;
use crate::query::spatial_query::SpatialQueryInterface;
use crate::{BevyMaterial, PION, PreFrameWait, get_time};
use bevy::pbr::ExtendedMaterial;
use bevy::prelude::Transform as BevyTransform;
use bevy::prelude::*;
use bevy_mod_openxr::helper_traits::{ToQuat, ToQuaternionf, ToVec3, ToVector3f};
use bevy_mod_openxr::resources::{OxrFrameState, Pipelined};
use bevy_mod_openxr::session::OxrSession;
use bevy_mod_xr::hands::{HandBone, HandSide, XrHandBoneEntities, XrHandBoneRadius};
use bevy_mod_xr::session::{XrPreDestroySession, XrSessionCreated, session_available};
use bevy_mod_xr::spaces::{XrPrimaryReferenceSpace, XrSpace, XrSpaceLocationFlags};
use bevy_sk::hand::GRADIENT_TEXTURE_HANDLE;
use binderbinder::binder_object::{BinderObject, BinderObjectRef, ToBinderObjectOrRef};
use color_eyre::eyre::Result;
use glam::{Mat4, Quat, Vec3};
use gluon_wire::{GluonSendError, Handler};
use openxr::{HandJointLocation, Posef, ReferenceSpaceType, SpaceLocationFlags};
use serde::{Deserialize, Serialize};
use stardust_xr_protocol::query::{InterfaceDependency, QueriedInterface, QueryableObjectRef};
use stardust_xr_protocol::spatial::SpatialRef as SpatialRefProxy;
use stardust_xr_protocol::spatial_query::{
	Point, PointsQuery, PointsQueryHandle, PointsQueryHandler, SpatialQueryGuard,
	SpatialQueryInterface as SpatialQueryInterfaceProxy,
};
use stardust_xr_protocol::suis::{
	Chirality, DatamapData, Finger, Hand, InputDataType, InputHandler, InputMethod,
	InputMethodHandler, Joint, SemanticData, SpatialData, Thumb,
};
use stardust_xr_protocol::types::{self, Timestamp, Vec3F};
use std::any::type_name;
use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use tokio::sync::RwLock;
use tracing::Instrument;
use zbus::Connection;

type HandHoldoutMaterial = ExtendedMaterial<BevyMaterial, HoldoutExtension>;

#[derive(Resource)]
pub struct HandRenderConfig {
	pub transparent: bool,
}

pub struct HandPlugin {
	pub transparent_hands: bool,
}
impl Plugin for HandPlugin {
	fn build(&self, app: &mut App) {
		app.insert_resource(HandRenderConfig {
			transparent: self.transparent_hands,
		});

		app.add_systems(PreFrameWait, update_hands.run_if(resource_exists::<Hands>));
		app.add_systems(XrSessionCreated, create_trackers);
		app.add_systems(XrPreDestroySession, destroy_trackers);
		app.add_systems(
			PostUpdate,
			update_hand_material.run_if(resource_exists::<Hands>),
		);
		app.add_systems(Startup, setup.run_if(session_available));
	}
}

fn update_hands(
	mut hands: ResMut<Hands>,
	session: Option<Res<OxrSession>>,
	state: Option<Res<OxrFrameState>>,
	ref_space: Option<Res<XrPrimaryReferenceSpace>>,
	mut materials: ResMut<Assets<BevyMaterial>>,
	mut joint_query: Query<(
		&mut BevyTransform,
		&mut XrSpaceLocationFlags,
		&mut XrHandBoneRadius,
	)>,
	joints_query: Query<&XrHandBoneEntities>,
	pipelined: Option<Res<Pipelined>>,
) {
	let (Some(session), Some(state), Some(ref_space)) = (session, state, ref_space) else {
		return;
	};
	let time = get_time(pipelined.is_some(), &state);
	if let Some(base_space) = hands.base_space.as_ref() {
		let pose = session.locate_space(
			&unsafe { XrSpace::from_raw(base_space.as_raw().into_raw()) },
			&ref_space,
			time,
		);
		if let Ok(pose) = pose
			&& pose.location_flags.contains(
				SpaceLocationFlags::POSITION_TRACKED | SpaceLocationFlags::ORIENTATION_TRACKED,
			) {
			hands
				.base_spatial
				.set_local_transform(Mat4::from_rotation_translation(
					pose.pose.orientation.to_quat(),
					pose.pose.position.to_vec3(),
				));
		}
	}
	let base_spatial = hands.base_spatial.get_ref().clone();
	hands.left.update(time, &mut materials, &base_spatial);
	hands.right.update(time, &mut materials, &base_spatial);
}

fn pinch_between(joint_1: &Joint, joint_2: &Joint) -> f32 {
	const PINCH_MAX: f32 = 0.11;
	const PINCH_ACTIVACTION_DISTANCE: f32 = 0.01;
	let combined_radius = joint_1.radius + joint_2.radius;
	let pinch_dist = joint_1
		.pose
		.position
		.mint::<Vec3>()
		.distance(joint_2.pose.position.mint())
		- combined_radius;
	(1.0 - ((pinch_dist - PINCH_ACTIVACTION_DISTANCE) / (PINCH_MAX - PINCH_ACTIVACTION_DISTANCE)))
		.clamp(0.0, 1.0)
}

fn create_trackers(session: Res<OxrSession>, mut hands: ResMut<Hands>) {
	let Ok(base_space) = (**session)
		.create_reference_space(ReferenceSpaceType::LOCAL, Posef::IDENTITY)
		.inspect_err(|err| error!("failed to create openxr local space: {err}"))
		.map(Arc::new)
	else {
		return;
	};
	hands.base_space = Some(base_space.clone());
	if let Ok(tracker) = session
		.create_hand_tracker(openxr::HandEXT::LEFT)
		.inspect_err(|err| error!("failed to create left hand tracker: {err}"))
		&& let Ok(method) = HandInputMethod::new(
			hands.base_spatial.get_ref().clone(),
			base_space.clone(),
			HandSide::Left,
			tracker,
		)
		.inspect_err(|err| error!("failed to create left hand input method: {err}"))
	{
		hands.left.method = Some(PION.register_object(method));
	}
	if let Ok(tracker) = session
		.create_hand_tracker(openxr::HandEXT::RIGHT)
		.inspect_err(|err| error!("failed to create right hand tracker: {err}"))
		&& let Ok(method) = HandInputMethod::new(
			hands.base_spatial.get_ref().clone(),
			base_space,
			HandSide::Right,
			tracker,
		)
		.inspect_err(|err| error!("failed to create right hand input method: {err}"))
	{
		hands.right.method = Some(PION.register_object(method));
	}
}

fn destroy_trackers(mut hands: ResMut<Hands>) {
	hands.left.method.take();
	hands.right.method.take();
}

#[derive(Component)]
struct CorrectHandMaterial;

fn update_hand_material(
	query: Query<(Entity, &HandSide), (With<XrHandBoneEntities>, Without<CorrectHandMaterial>)>,
	mut cmds: Commands,
	hands: Res<Hands>,
) {
	for (entity, side) in &query {
		let hand = match side {
			HandSide::Left => &hands.left,
			HandSide::Right => &hands.right,
		};
		match &hand.material {
			HandMaterial::Normal(handle) => {
				cmds.entity(entity)
					.insert(MeshMaterial3d(handle.clone()))
					.insert(CorrectHandMaterial);
			}
			HandMaterial::Holdout(handle) => {
				cmds.entity(entity)
					.insert(MeshMaterial3d(handle.clone()))
					.insert(CorrectHandMaterial);
			}
		}
	}
}

fn setup(
	mut cmds: Commands,
	mut materials: ResMut<Assets<BevyMaterial>>,
	mut holdout_materials: ResMut<Assets<HandHoldoutMaterial>>,
	hand_config: Res<HandRenderConfig>,
) {
	let base_spatial = SpatialObject::new(None, Mat4::IDENTITY);
	cmds.insert_resource(Hands {
		left: OxrHandInput::new(
			HandSide::Left,
			base_spatial.get_ref(),
			&mut materials,
			&mut holdout_materials,
			&hand_config,
		)
		.unwrap(),
		right: OxrHandInput::new(
			HandSide::Right,
			base_spatial.get_ref(),
			&mut materials,
			&mut holdout_materials,
			&hand_config,
		)
		.unwrap(),
		base_space: None,
		base_spatial,
	});
}

fn convert_joint(joint: HandJointLocation) -> Joint {
	Joint {
		pose: types::Posef {
			position: joint.pose.position.to_vec3().into(),
			orientation: joint.pose.orientation.to_quat().into(),
		},
		radius: joint.radius,
		distance: 0.0,
	}
}

#[derive(Resource)]
struct Hands {
	left: OxrHandInput,
	right: OxrHandInput,
	base_space: Option<Arc<openxr::Space>>,
	base_spatial: BinderObjectRef<SpatialObject>,
}

#[derive(Debug, Default, Deserialize, Serialize, Clone, Copy)]
struct HandDatamap {
	pinch_strength: f32,
	grab_strength: f32,
}

enum HandMaterial {
	Normal(Handle<BevyMaterial>),
	Holdout(Handle<HandHoldoutMaterial>),
}

// ── OxrHandInput ──────────────────────────────────────────────────────────────

pub struct OxrHandInput {
	palm_spatial: BinderObjectRef<SpatialObject>,
	side: HandSide,
	method: Option<BinderObject<HandInputMethod>>,
	captured: bool,
	material: HandMaterial,
	was_enabled: bool,
}

impl OxrHandInput {
	pub fn new(
		side: HandSide,
		base_space: &BinderObjectRef<SpatialRef>,
		materials: &mut Assets<BevyMaterial>,
		holdout_materials: &mut Assets<HandHoldoutMaterial>,
		hand_config: &HandRenderConfig,
	) -> Result<Self> {
		let palm_spatial = SpatialObject::new(Some(&***base_space), Mat4::IDENTITY);

		let material = if hand_config.transparent {
			HandMaterial::Holdout(holdout_materials.add(HandHoldoutMaterial {
				base: BevyMaterial::default(),
				extension: HoldoutExtension {},
			}))
		} else {
			HandMaterial::Normal(materials.add(BevyMaterial {
				base_color: Srgba::new(1.0, 1.0, 1.0, 1.0).into(),
				alpha_mode: AlphaMode::Blend,
				base_color_texture: Some(GRADIENT_TEXTURE_HANDLE),
				perceptual_roughness: 1.0,
				..default()
			}))
		};

		Ok(OxrHandInput {
			palm_spatial,
			side,
			material,
			captured: false,
			was_enabled: false,
			method: None,
		})
	}

	pub fn set_enabled(&self, _enabled: bool) {}

	fn update(
		&mut self,
		time: openxr::Time,
		materials: &mut ResMut<Assets<BevyMaterial>>,
		base_space: &BinderObjectRef<SpatialRef>,
	) {
		let new_hand = self
			.method
			.as_ref()
			.and_then(|m| m.locate_hand(base_space, time));

		let is_tracked = new_hand.is_some();
		self.set_enabled(is_tracked);

		if let Some(new_hand) = &new_hand {
			self.palm_spatial
				.set_local_transform(Mat4::from_rotation_translation(
					new_hand.palm.pose.orientation.mint(),
					new_hand.palm.pose.position.mint(),
				));

			if let Some(method) = self.method.as_ref()
				&& let Some(handle) = method.query_handle.get()
			{
				_ = handle.update_points(
					[
						new_hand.thumb.tip,
						new_hand.index.tip,
						new_hand.middle.tip,
						new_hand.ring.tip,
					]
					.into_iter()
					.map(|v| Point {
						point: v.pose.position,
						margin: v.radius + 0.5,
					})
					.collect(),
				);
			}
		}

		let Some(method) = self.method.as_ref() else {
			return;
		};

		let new_datamap = new_hand.as_ref().map(|hand| HandDatamap {
			pinch_strength: pinch_between(&hand.thumb.tip, &hand.index.tip),
			grab_strength: pinch_between(&hand.ring.tip, &hand.ring.metacarpal),
		});

		*method.hand.blocking_write() = new_hand;
		if let Some(dm) = new_datamap {
			*method.datamap.blocking_write() = dm;
		}

		if let HandMaterial::Normal(material_handle) = &self.material {
			let captured = method.capture.blocking_read().is_some();
			if captured && !self.captured {
				materials.get_mut(material_handle).unwrap().base_color =
					Srgba::rgb(0., 1., 0.75).into();
			} else if self.captured && !captured {
				materials.get_mut(material_handle).unwrap().base_color =
					Srgba::rgb(1., 1.0, 1.0).into();
			}
			self.captured = captured;
		}

		let input_method = InputMethod::from_handler(method);
		let ts = method
			.base_space
			.instance()
			.xr_to_timestamp(time)
			.unwrap_or_else(Timestamp::now);
		let sender = method.sender.clone();
		sender.send(&***method, input_method, ts);
	}
}

// ── HandInputMethod ───────────────────────────────────────────────────────────

#[derive(Debug, Handler)]
struct HandInputMethod {
	side: HandSide,
	base_space: DebugWrapper<Arc<openxr::Space>>,
	base_spatial: BinderObjectRef<SpatialRef>,
	tracker: DebugWrapper<openxr::HandTracker>,
	_query: BinderObject<PointsQueryCache>,
	sender: Arc<InputSender<f32>>,
	hand: RwLock<Option<Hand>>,
	datamap: RwLock<HandDatamap>,
	capture: RwLock<Option<InputHandler>>,
	query_handle: Arc<OnceLock<PointsQueryHandle>>,
}

impl HandInputMethod {
	fn new(
		base_spatial: BinderObjectRef<SpatialRef>,
		base_space: Arc<openxr::Space>,
		side: HandSide,
		tracker: openxr::HandTracker,
	) -> Result<Self, GluonSendError> {
		let (query_cache, objects_arc) = QueryCache::new();
		let sender = Arc::new(InputSender::new(objects_arc));

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
				if let Ok(handle) = handle {
					query_handle.set(handle);
				}
			}
		});

		Ok(Self {
			side,
			base_space: base_space.into(),
			base_spatial,
			tracker: tracker.into(),
			_query: query,
			sender,
			hand: RwLock::new(None),
			datamap: RwLock::new(HandDatamap::default()),
			capture: RwLock::new(None),
			query_handle,
		})
	}

	fn locate_hand(
		&self,
		relative_to: &BinderObjectRef<SpatialRef>,
		time: openxr::Time,
	) -> Option<Hand> {
		let joints = {
			let mat = Spatial::space_to_space_matrix(Some(&self.base_spatial), Some(relative_to));
			self.base_space
				.locate_hand_joints(&self.tracker, time)
				.inspect_err(|err| error!("Error while locating hand joints: {err}"))
				.ok()
				.flatten()
				.map(|joints| {
					joints.map(|mut j| {
						if j.location_flags
							.contains(SpaceLocationFlags::POSITION_VALID)
						{
							j.pose.position = mat
								.transform_point3(j.pose.position.to_vec3())
								.to_vector3f();
						}
						if j.location_flags
							.contains(SpaceLocationFlags::ORIENTATION_VALID)
						{
							j.pose.orientation = (mat.to_scale_rotation_translation().1
								* j.pose.orientation.to_quat())
							.to_quaternionf();
						}
						j
					})
				})
		};
		let real_hand = true;
		let is_tracked = real_hand
			&& joints.is_some_and(|v| {
				v.iter().all(|v| {
					v.location_flags.contains(
						SpaceLocationFlags::POSITION_VALID | SpaceLocationFlags::POSITION_TRACKED,
					) || v.location_flags.contains(
						SpaceLocationFlags::ORIENTATION_VALID
							| SpaceLocationFlags::ORIENTATION_TRACKED,
					)
				})
			});
		if is_tracked {
			let joints = joints.unwrap();
			Some(Hand {
				chirality: match self.side {
					HandSide::Left => Chirality::Left,
					HandSide::Right => Chirality::Right,
				},
				thumb: Thumb {
					tip: convert_joint(joints[HandBone::ThumbTip as usize]),
					distal: convert_joint(joints[HandBone::ThumbDistal as usize]),
					proximal: convert_joint(joints[HandBone::ThumbProximal as usize]),
					metacarpal: convert_joint(joints[HandBone::ThumbMetacarpal as usize]),
				},
				index: Finger {
					tip: convert_joint(joints[HandBone::IndexTip as usize]),
					distal: convert_joint(joints[HandBone::IndexDistal as usize]),
					intermediate: convert_joint(joints[HandBone::IndexIntermediate as usize]),
					proximal: convert_joint(joints[HandBone::IndexProximal as usize]),
					metacarpal: convert_joint(joints[HandBone::IndexMetacarpal as usize]),
				},
				middle: Finger {
					tip: convert_joint(joints[HandBone::MiddleTip as usize]),
					distal: convert_joint(joints[HandBone::MiddleDistal as usize]),
					intermediate: convert_joint(joints[HandBone::MiddleIntermediate as usize]),
					proximal: convert_joint(joints[HandBone::MiddleProximal as usize]),
					metacarpal: convert_joint(joints[HandBone::MiddleMetacarpal as usize]),
				},
				ring: Finger {
					tip: convert_joint(joints[HandBone::RingTip as usize]),
					distal: convert_joint(joints[HandBone::RingDistal as usize]),
					intermediate: convert_joint(joints[HandBone::RingIntermediate as usize]),
					proximal: convert_joint(joints[HandBone::RingProximal as usize]),
					metacarpal: convert_joint(joints[HandBone::RingMetacarpal as usize]),
				},
				little: Finger {
					tip: convert_joint(joints[HandBone::LittleTip as usize]),
					distal: convert_joint(joints[HandBone::LittleDistal as usize]),
					intermediate: convert_joint(joints[HandBone::LittleIntermediate as usize]),
					proximal: convert_joint(joints[HandBone::LittleProximal as usize]),
					metacarpal: convert_joint(joints[HandBone::LittleMetacarpal as usize]),
				},
				palm: convert_joint(joints[HandBone::Palm as usize]),
				wrist: convert_joint(joints[HandBone::Wrist as usize]),
				elbow: None,
			})
		} else {
			None
		}
	}
}

impl InputSource for HandInputMethod {
	type QueryValue = f32;

	fn order_handlers_and_captures(
		&self,
		objects: &HashMap<QueryableObjectRef, CachedObject<f32>>,
		capture_requests: &HashSet<InputHandler>,
	) -> (Vec<InputHandler>, Option<InputHandler>) {
		let hand = *self.hand.blocking_read();
		let Some(hand) = hand else {
			self.capture.blocking_write().take();
			return (vec![], None);
		};

		let current_capture = self.capture.blocking_read().clone();
		let capture = if let Some(cap) = current_capture {
			if objects.values().any(|e| e.handler == cap) {
				Some(cap)
			} else {
				self.capture.blocking_write().take();
				None
			}
		} else {
			let promoted = capture_requests
				.iter()
				.find(|r| objects.values().any(|e| &e.handler == *r))
				.cloned();
			if let Some(ref p) = promoted {
				*self.capture.blocking_write() = Some(p.clone());
			}
			promoted
		};

		if let Some(ref cap) = capture {
			let handlers: Vec<_> = objects
				.values()
				.filter(|e| &e.handler == cap)
				.map(|e| e.handler.clone())
				.collect();
			return (handlers, capture);
		}

		let mut order: Vec<_> = objects
			.values()
			.map(|e| {
				let dist = hand_sort_distance(&self.base_spatial, &e.field.data, &hand);
				(dist, e.handler.clone())
			})
			.collect();
		order.sort_by(|(d1, _), (d2, _)| d1.total_cmp(d2));
		(order.into_iter().map(|(_, h)| h).collect(), None)
	}

	fn spatial_data(&self, handler_spatial: &SpatialRef, handler_field: &Field) -> SpatialData {
		let hand = (*self.hand.blocking_read()).unwrap();
		let localized = localize_hand(&self.base_spatial, &hand, handler_spatial, handler_field);
		let distance = hand_real_distance(&localized);
		SpatialData {
			input: InputDataType::Hand { data: localized },
			distance,
		}
	}

	fn datamap(
		&self,
		suggested_bindings: &HashMap<String, Vec<String>>,
	) -> HashMap<String, DatamapData> {
		let data = *self.datamap.blocking_read();
		build_hand_datamap(&data, suggested_bindings)
	}
}

impl InputMethodHandler for HandInputMethod {
	async fn request_capture(&self, _ctx: gluon_wire::GluonCtx, handler: InputHandler) {
		self.sender.request_capture(handler).await;
	}

	async fn release_capture(&self, _ctx: gluon_wire::GluonCtx, handler: InputHandler) {
		self.sender.release_capture(&handler).await;
		let mut cap = self.capture.write().await;
		if cap.as_ref() == Some(&handler) {
			cap.take();
		}
	}

	async fn get_spatial_data(
		&self,
		_ctx: gluon_wire::GluonCtx,
		handler: InputHandler,
		time: Timestamp,
	) -> Option<SpatialData> {
		let cap = self.capture.read().await.clone();
		if cap.as_ref().is_some_and(|c| c != &handler) {
			return None;
		}
		self.hand.read().await.as_ref()?;
		let objects = self.sender.cache.read().await;
		let entry = objects.values().find(|e| e.handler == handler)?;
		let hand = self.locate_hand(
			&entry.spatial,
			self.base_space.instance().timestamp_to_xr(time)?,
		)?;
		let hand = localize_hand(&entry.spatial, &hand, &entry.spatial, &entry.field.data);
		Some(SpatialData {
			distance: hand_real_distance(&hand),
			input: InputDataType::Hand { data: hand },
		})
	}
}

// ── Free functions ────────────────────────────────────────────────────────────

fn hand_sort_distance(hand_space: &SpatialRef, field: &Field, hand: &Hand) -> f32 {
	let thumb_tip_distance = field
		.sample(hand_space, hand.thumb.tip.pose.position.mint())
		.distance;
	let index_tip_distance = field
		.sample(hand_space, hand.index.tip.pose.position.mint())
		.distance;
	let middle_tip_distance = field
		.sample(hand_space, hand.middle.tip.pose.position.mint())
		.distance;
	let ring_tip_distance = field
		.sample(hand_space, hand.ring.tip.pose.position.mint())
		.distance;

	(thumb_tip_distance * 0.3)
		+ (index_tip_distance * 0.4)
		+ (middle_tip_distance * 0.15)
		+ (ring_tip_distance * 0.15)
}

/// assumes joint distances are populated
fn hand_real_distance(hand: &Hand) -> f32 {
	let get_dist = |joint: &Joint| joint.distance - joint.radius;

	get_dist(&hand.thumb.tip)
		.min(get_dist(&hand.index.tip))
		.min(get_dist(&hand.middle.tip))
		.min(get_dist(&hand.ring.tip))
}

fn localize_hand(from: &Spatial, hand: &Hand, to: &Spatial, field: &Field) -> Hand {
	Hand {
		chirality: hand.chirality,
		thumb: Thumb {
			tip: transform_joint(from, to, field, &hand.thumb.tip),
			distal: transform_joint(from, to, field, &hand.thumb.distal),
			proximal: transform_joint(from, to, field, &hand.thumb.proximal),
			metacarpal: transform_joint(from, to, field, &hand.thumb.metacarpal),
		},
		index: transform_finger(from, to, field, &hand.index),
		middle: transform_finger(from, to, field, &hand.middle),
		ring: transform_finger(from, to, field, &hand.ring),
		little: transform_finger(from, to, field, &hand.little),
		palm: transform_joint(from, to, field, &hand.palm),
		wrist: transform_joint(from, to, field, &hand.wrist),
		elbow: hand
			.elbow
			.as_ref()
			.map(|j| transform_joint(from, to, field, j)),
	}
}

fn transform_finger(from: &Spatial, to: &Spatial, field: &Field, finger: &Finger) -> Finger {
	Finger {
		tip: transform_joint(from, to, field, &finger.tip),
		distal: transform_joint(from, to, field, &finger.distal),
		intermediate: transform_joint(from, to, field, &finger.intermediate),
		proximal: transform_joint(from, to, field, &finger.proximal),
		metacarpal: transform_joint(from, to, field, &finger.metacarpal),
	}
}

fn transform_joint(from: &Spatial, to: &Spatial, field: &Field, joint: &Joint) -> Joint {
	let mat = Spatial::space_to_space_matrix(Some(from), Some(to));
	let (_, rot, _) = mat.to_scale_rotation_translation();
	Joint {
		pose: types::Posef {
			position: mat.transform_point3a(joint.pose.position.mint()).into(),
			orientation: (rot * joint.pose.orientation.mint::<Quat>()).into(),
		},
		radius: joint.radius,
		distance: field.sample(from, joint.pose.position.mint()).distance,
	}
}

fn build_hand_datamap(
	data: &HandDatamap,
	suggested_bindings: &HashMap<String, Vec<String>>,
) -> HashMap<String, DatamapData> {
	let mut grab_bindings = HashSet::new();
	let mut pinch_bindings = HashSet::new();
	for (name, bindings) in suggested_bindings {
		for binding in bindings {
			if binding == "pinch_strength" || binding == "pinch" {
				pinch_bindings.insert(name.clone());
			}
			if binding == "grab_strength" || binding == "grab" {
				grab_bindings.insert(name.clone());
			}
		}
	}

	let mut map = HashMap::new();
	map.insert(
		"pinch_strength".to_string(),
		DatamapData::Float {
			value: data.pinch_strength,
		},
	);
	map.insert(
		"grab_strength".to_string(),
		DatamapData::Float {
			value: data.grab_strength,
		},
	);
	for binding in grab_bindings {
		if let DatamapData::Float { value } = map
			.entry(binding)
			.or_insert(DatamapData::Float { value: 0.0 })
		{
			*value = data.grab_strength.max(*value);
		}
	}
	for binding in pinch_bindings {
		if let DatamapData::Float { value } = map
			.entry(binding)
			.or_insert(DatamapData::Float { value: 0.0 })
		{
			*value = data.pinch_strength.max(*value);
		}
	}
	map
}

// ── Helpers ───────────────────────────────────────────────────────────────────

#[derive(Deref, DerefMut)]
struct DebugWrapper<T>(T);
impl<T> From<T> for DebugWrapper<T> {
	fn from(value: T) -> Self {
		Self(value)
	}
}
impl<T> Debug for DebugWrapper<T> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_tuple("DebugWrapper")
			.field(&type_name::<T>())
			.finish()
	}
}

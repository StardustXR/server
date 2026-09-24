use super::{
	CachedHandler, DatamapBuilder, FrameDriver, InputMethod as InputMethodNode, InputMethodHelper,
	order_by_distance, spawn_frame_driver,
};
use crate::nodes::ProxyExt;
use crate::nodes::drawable::model::HoldoutExtension;
use crate::nodes::fields::Field;
use crate::nodes::spatial::{Spatial, SpatialObject, SpatialRef};
use crate::objects::{DebugWrapper, Tracked};
use crate::openxr_helpers::ConvertTimespec;
use crate::{BevyMaterial, PreFrameWait, get_time};
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
use color_eyre::eyre::Result;
use glam::{Mat4, Quat, Vec3};
use gluon_ipc::{Handler, LocalRef, Node, RefExt};
use openxr::{HandJointLocation, Posef, ReferenceSpaceType, SpaceLocationFlags};
use serde::{Deserialize, Serialize};
use stardust_xr_protocol::field::FieldSample;
use stardust_xr_protocol::query::QueryableId;
use stardust_xr_protocol::spatial::{Spatial as SpatialProxy, SpatialRef as SpatialRefProxy};
use stardust_xr_protocol::spatial_query::{Point, PointsQueryHandle};
use stardust_xr_protocol::suis::{
	Chirality, DatamapData, Finger, Hand, InputDataType, InputHandler, Joint, Thumb,
};
use stardust_xr_protocol::types::{self, Timestamp};
use stardust_xr_server_wboit::{WboitExtension, WboitMaterial};
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
	mut materials: ResMut<Assets<WboitMaterial>>,
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
	let pinch_dist =
		Vec3::from(joint_1.pose.position).distance(joint_2.pose.position.into()) - combined_radius;
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
	let base_spatial = hands.base_spatial.get_ref().clone();
	for (side, hand_ext) in [
		(HandSide::Left, openxr::HandEXT::LEFT),
		(HandSide::Right, openxr::HandEXT::RIGHT),
	] {
		let Ok(tracker) = session
			.create_hand_tracker(hand_ext)
			.inspect_err(|err| error!("failed to create {side:?} hand tracker: {err}"))
		else {
			continue;
		};
		let helper = HandInputMethod::new(base_spatial.clone(), base_space.clone(), side, tracker);
		let Ok((method, _, query_handle)) = InputMethodNode::new_points(
			helper,
			(**base_spatial).clone(),
			base_spatial.proxy().clone(),
			vec![],
		)
		.inspect_err(|err| error!("failed to create {side:?} hand input method: {err}")) else {
			continue;
		};
		let hand = match side {
			HandSide::Left => &mut hands.left,
			HandSide::Right => &mut hands.right,
		};
		hand.tracked.get_mut_data_blocking().method = Arc::downgrade(method.handler());
		hand.driver = Some(spawn_frame_driver(&method));
		hand.query_handle = Some(query_handle);
		hand.method = Some(method);
	}
}

fn destroy_trackers(mut hands: ResMut<Hands>) {
	hands.left.method.take();
	hands.right.method.take();
	hands.left.driver.take();
	hands.right.driver.take();
	hands.left.query_handle.take();
	hands.right.query_handle.take();
	hands.left.tracked.get_mut_data_blocking().method = Weak::new();
	hands.right.tracked.get_mut_data_blocking().method = Weak::new();
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
	mut materials: ResMut<Assets<WboitMaterial>>,
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
	base_spatial: LocalRef<SpatialProxy, SpatialObject>,
}

#[derive(Debug, Default, Deserialize, Serialize, Clone, Copy)]
struct HandDatamap {
	pinch_strength: f32,
	grab_strength: f32,
}

enum HandMaterial {
	Normal(Handle<WboitMaterial>),
	Holdout(Handle<HandHoldoutMaterial>),
}

// ── OxrHandInput ──────────────────────────────────────────────────────────────

#[derive(Debug)]
struct OxrHandInputTrackedState {
	method: Weak<InputMethodNode<HandInputMethod>>,
	palm_spatial: LocalRef<SpatialProxy, SpatialObject>,
}
impl OxrHandInputTrackedState {
	fn get_pose(&self, relative_to: &Spatial, at: Timestamp) -> (Option<types::Posef>, bool) {
		if let Some(method) = self.method.upgrade() {
			let Some(time) = method.base_space.instance().timestamp_to_xr(at) else {
				return (None, false);
			};
			let Some(hand) = method.locate_hand(relative_to, time) else {
				return (None, false);
			};
			(Some(hand.palm.pose), true)
		} else {
			let mat = crate::nodes::spatial::Spatial::space_to_space_matrix(
				Some(&self.palm_spatial),
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

pub struct OxrHandInput {
	palm_spatial: LocalRef<SpatialProxy, SpatialObject>,
	side: HandSide,
	method: Option<Node<InputMethodNode<HandInputMethod>>>,
	driver: Option<FrameDriver>,
	query_handle: Option<Arc<OnceLock<PointsQueryHandle>>>,
	captured: bool,
	material: HandMaterial,
	tracked: Tracked<OxrHandInputTrackedState>,
	was_enabled: bool,
}

impl OxrHandInput {
	pub fn new(
		side: HandSide,
		base_space: &LocalRef<SpatialRefProxy, SpatialRef>,
		materials: &mut Assets<WboitMaterial>,
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
			HandMaterial::Normal(materials.add(WboitMaterial {
				base: BevyMaterial {
					base_color: Srgba::new(1.0, 1.0, 1.0, 1.0).into(),
					alpha_mode: AlphaMode::Blend,
					base_color_texture: Some(GRADIENT_TEXTURE_HANDLE),
					perceptual_roughness: 1.0,
					..default()
				},
				extension: WboitExtension {},
			}))
		};
		let pion_path = match side {
			HandSide::Left => "stardust-hand/left",
			HandSide::Right => "stardust-hand/right",
		};
		let tracked = Tracked::new(
			palm_spatial.get_ref().proxy().clone(),
			OxrHandInputTrackedState::get_pose,
			false,
			pion_path,
			OxrHandInputTrackedState {
				method: Weak::new(),
				palm_spatial: palm_spatial.clone(),
			},
		)
		.unwrap();
		Ok(OxrHandInput {
			palm_spatial,
			side,
			material,
			captured: false,
			was_enabled: false,
			method: None,
			driver: None,
			query_handle: None,
			tracked,
		})
	}

	pub fn set_enabled(&self, enabled: bool) {
		self.tracked.tracked_blocking(enabled);
	}

	fn update(
		&mut self,
		time: openxr::Time,
		materials: &mut ResMut<Assets<WboitMaterial>>,
		base_space: &LocalRef<SpatialRefProxy, SpatialRef>,
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
					new_hand.palm.pose.orientation.into(),
					new_hand.palm.pose.position.into(),
				));

			if let Some(handle) = self.query_handle.as_ref().and_then(|h| h.get()) {
				_ = handle.update(
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
					.collect::<Vec<_>>(),
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

		let ts = method
			.base_space
			.instance()
			.xr_to_timestamp(time)
			.unwrap_or_else(Timestamp::now);
		*method.hand.blocking_write() = new_hand.map(|hand| (ts, hand));
		if let Some(dm) = new_datamap {
			*method.datamap.blocking_write() = dm;
		}

		if let HandMaterial::Normal(material_handle) = &self.material {
			let captured = method.active_capture_blocking().is_some();
			if captured && !self.captured {
				materials.get_mut(material_handle).unwrap().base.base_color =
					Srgba::rgb(0., 1., 0.75).into();
			} else if self.captured && !captured {
				materials.get_mut(material_handle).unwrap().base.base_color =
					Srgba::rgb(1., 1.0, 1.0).into();
			}
			self.captured = captured;
		}

		if let Some(driver) = self.driver.as_ref() {
			driver.frame(ts);
		}
	}
}

// ── HandInputMethod ───────────────────────────────────────────────────────────

#[derive(Debug)]
struct HandInputMethod {
	side: HandSide,
	base_space: DebugWrapper<Arc<openxr::Space>>,
	base_spatial: LocalRef<SpatialRefProxy, SpatialRef>,
	tracker: DebugWrapper<openxr::HandTracker>,
	/// the frame's hand and the moment it was located for, so a pull for that same moment
	/// doesn't relocate
	hand: RwLock<Option<(Timestamp, Hand)>>,
	datamap: RwLock<HandDatamap>,
}

impl HandInputMethod {
	fn new(
		base_spatial: LocalRef<SpatialRefProxy, SpatialRef>,
		base_space: Arc<openxr::Space>,
		side: HandSide,
		tracker: openxr::HandTracker,
	) -> Self {
		Self {
			side,
			base_space: base_space.into(),
			base_spatial,
			tracker: tracker.into(),
			hand: RwLock::new(None),
			datamap: RwLock::new(HandDatamap::default()),
		}
	}

	async fn hand_at(&self, time: Timestamp) -> Option<Hand> {
		match *self.hand.read().await {
			Some((at, hand)) if at == time => Some(hand),
			_ => self.locate_hand(
				&self.base_spatial,
				self.base_space.instance().timestamp_to_xr(time)?,
			),
		}
	}

	fn locate_hand(&self, relative_to: &Spatial, time: openxr::Time) -> Option<Hand> {
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

impl InputMethodHelper for HandInputMethod {
	type QueryValue = FieldSample;

	async fn order_handlers_and_captures(
		&self,
		handlers: &HashMap<QueryableId, CachedHandler<FieldSample>>,
		capture_requests: &HashSet<InputHandler>,
		active_capture: Option<&InputHandler>,
	) -> (Vec<InputHandler>, Option<InputHandler>) {
		let Some((_, hand)) = *self.hand.read().await else {
			return (vec![], active_capture.cloned());
		};
		order_by_distance(handlers, capture_requests, active_capture, |e| {
			Some(hand_sort_distance(&self.base_spatial, &e.field, &hand).abs())
		})
	}

	async fn input_data(&self, time: Timestamp) -> Option<InputDataType> {
		Some(InputDataType::Hand {
			data: self.hand_at(time).await?,
		})
	}

	async fn datamap(&self) -> HashMap<String, DatamapData> {
		build_hand_datamap(&*self.datamap.read().await)
	}
}

// ── Free functions ────────────────────────────────────────────────────────────

fn hand_sort_distance(hand_space: &SpatialRef, field: &Field, hand: &Hand) -> f32 {
	let thumb_tip_distance = field
		.sample(hand_space, hand.thumb.tip.pose.position.into())
		.distance;
	let index_tip_distance = field
		.sample(hand_space, hand.index.tip.pose.position.into())
		.distance;
	let middle_tip_distance = field
		.sample(hand_space, hand.middle.tip.pose.position.into())
		.distance;
	let ring_tip_distance = field
		.sample(hand_space, hand.ring.tip.pose.position.into())
		.distance;

	(thumb_tip_distance * 0.3)
		+ (index_tip_distance * 0.4)
		+ (middle_tip_distance * 0.15)
		+ (ring_tip_distance * 0.15)
}

fn build_hand_datamap(data: &HandDatamap) -> HashMap<String, DatamapData> {
	DatamapBuilder::default()
		.f32("pinch_strength", data.pinch_strength)
		.f32("grab_strength", data.grab_strength)
		.build()
}

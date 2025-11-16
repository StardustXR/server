use crate::core::client::INTERNAL_CLIENT;
use crate::nodes::OwnedNode;
use crate::nodes::fields::{Field, FieldTrait};
use crate::nodes::input::{Finger, INPUT_HANDLER_REGISTRY, InputDataType, InputHandler, Thumb};
use crate::nodes::{
	Node,
	input::{Hand, InputMethod, Joint},
	spatial::Spatial,
};
use crate::nodes::drawable::model::HoldoutExtension;
use crate::objects::{AsyncTracked, ObjectHandle, SpatialRef, Tracked};
use crate::{BevyMaterial, DbusConnection, ObjectRegistryRes, PreFrameWait, get_time};
use bevy::prelude::Transform as BevyTransform;
use bevy::prelude::*;
use bevy::pbr::ExtendedMaterial;
use bevy_mod_openxr::helper_traits::{ToQuat, ToVec3};
use bevy_mod_openxr::resources::{OxrFrameState, Pipelined};
use bevy_mod_openxr::session::OxrSession;
use bevy_mod_xr::hands::{HandBone, HandSide, XrHandBoneEntities, XrHandBoneRadius};
use bevy_mod_xr::session::{XrPreDestroySession, XrSessionCreated, session_available};
use bevy_mod_xr::spaces::{XrPrimaryReferenceSpace, XrSpaceLocationFlags};
use bevy_sk::hand::GRADIENT_TEXTURE_HANDLE;
use color_eyre::eyre::Result;
use glam::{Mat4, Quat, Vec3};
use openxr::{HandJointLocation, SpaceLocationFlags};
use serde::{Deserialize, Serialize};
use stardust_xr::values::Datamap;
use std::sync::Arc;
use zbus::Connection;

use super::{CaptureManager, get_sorted_handlers};

// Holdout material for transparent hands (passthrough)
type HandHoldoutMaterial = ExtendedMaterial<BevyMaterial, HoldoutExtension>;

#[derive(Resource)]
pub struct HandRenderConfig {
	pub transparent: bool,
}

pub struct HandPlugin;
impl Plugin for HandPlugin {
	fn build(&self, app: &mut App) {
		app.add_plugins(MaterialPlugin::<HandHoldoutMaterial>::default());
		
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
		hands.left.tracked.set_tracked(false);
		hands.right.tracked.set_tracked(false);
		return;
	};
	let get_joints = |hand: &mut OxrHandInput| -> Option<openxr::HandJointLocations> {
		let Some(tracker) = hand.tracker.as_ref() else {
			hand.input.spatial.node().unwrap().set_enabled(false);
			hand.tracked.set_tracked(false);
			return None;
		};
		let time = get_time(pipelined.is_some(), &state);
		session
			.locate_hand_joints(tracker, &ref_space, time)
			.inspect_err(|err| error!("Error while locating hand joints"))
			.ok()
			.flatten()
	};
	let joints_left = get_joints(&mut hands.left);
	let joints_right = get_joints(&mut hands.right);
	hands.left.update(joints_left.as_ref(), &mut materials);
	hands.right.update(joints_right.as_ref(), &mut materials);
}

fn pinch_between(joint_1: &Joint, joint_2: &Joint) -> f32 {
	const PINCH_MAX: f32 = 0.11;
	const PINCH_ACTIVACTION_DISTANCE: f32 = 0.01;
	let combined_radius = joint_1.radius + joint_2.radius;
	let pinch_dist =
		Vec3::from(joint_1.position).distance(Vec3::from(joint_2.position)) - combined_radius;
	(1.0 - ((pinch_dist - PINCH_ACTIVACTION_DISTANCE) / (PINCH_MAX - PINCH_ACTIVACTION_DISTANCE)))
		.clamp(0.0, 1.0)
}

fn create_trackers(session: Res<OxrSession>, mut hands: ResMut<Hands>) {
	hands.left.tracker = session
		.create_hand_tracker(openxr::HandEXT::LEFT)
		.inspect_err(|err| error!("failed to create left hand tracker"))
		.ok();
	hands.right.tracker = session
		.create_hand_tracker(openxr::HandEXT::RIGHT)
		.inspect_err(|err| error!("failed to create right hand tracker"))
		.ok();
}
fn destroy_trackers(mut hands: ResMut<Hands>) {
	hands.left.tracker.take();
	hands.right.tracker.take();
}
#[derive(Component)]
struct CorrectHandMaterial;
fn update_hand_material(
	query: Query<
		(Entity, &HandSide),
		(
			With<XrHandBoneEntities>,
			Without<CorrectHandMaterial>,
		),
	>,
	mut cmds: Commands,
	hands: Res<Hands>,
) {
	for (entity, side) in &query {
		let hand = match side {
			HandSide::Left => &hands.left,
			HandSide::Right => &hands.right,
		};
		
		// Remove any existing materials first
		cmds.entity(entity)
			.remove::<MeshMaterial3d<BevyMaterial>>()
			.remove::<MeshMaterial3d<HandHoldoutMaterial>>();
		
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
	connection: Res<DbusConnection>,
	mut cmds: Commands,
	mut materials: ResMut<Assets<BevyMaterial>>,
	mut holdout_materials: ResMut<Assets<HandHoldoutMaterial>>,
	hand_config: Res<HandRenderConfig>,
) {
	tokio::task::spawn({
		let connection = connection.clone();
		async move {
			connection
				.request_name("org.stardustxr.Hands")
				.await
				.unwrap();
		}
	});
	cmds.insert_resource(Hands {
		left: OxrHandInput::new(&connection, HandSide::Left, &mut materials, &mut holdout_materials, hand_config.transparent).unwrap(),
		right: OxrHandInput::new(&connection, HandSide::Right, &mut materials, &mut holdout_materials, hand_config.transparent).unwrap(),
	});
}

fn convert_joint(joint: HandJointLocation) -> Joint {
	Joint {
		position: joint.pose.position.to_vec3().into(),
		rotation: joint.pose.orientation.to_quat().into(),
		radius: joint.radius,
		distance: 0.0,
	}
}

#[derive(Resource)]
struct Hands {
	left: OxrHandInput,
	right: OxrHandInput,
}

#[derive(Default, Deserialize, Serialize)]
struct HandDatamap {
	pinch_strength: f32,
	grab_strength: f32,
}

enum HandMaterial {
	Normal(Handle<BevyMaterial>),
	Holdout(Handle<HandHoldoutMaterial>),
}

pub struct OxrHandInput {
	_node: OwnedNode,
	palm_spatial: Arc<Spatial>,
	palm_object: ObjectHandle<SpatialRef>,
	side: HandSide,
	input: Arc<InputMethod>,
	capture_manager: CaptureManager,
	datamap: HandDatamap,
	tracked: AsyncTracked,
	tracker: Option<openxr::HandTracker>,
	captured: bool,
	material: HandMaterial,
}
impl OxrHandInput {
	pub fn new(
		connection: &Connection,
		side: HandSide,
		materials: &mut Assets<BevyMaterial>,
		holdout_materials: &mut Assets<HandHoldoutMaterial>,
		transparent: bool,
	) -> Result<Self> {
		let (palm_spatial, palm_object) = SpatialRef::create(
			connection,
			&("/org/stardustxr/Hand/".to_string()
				+ match side {
					HandSide::Left => "left",
					HandSide::Right => "right",
				} + "/palm"),
		);
		let tracked = AsyncTracked::new(
			connection,
			&("/org/stardustxr/Hand/".to_string()
				+ match side {
					HandSide::Left => "left",
					HandSide::Right => "right",
				}),
		);
		let node = Node::generate(&INTERNAL_CLIENT, false).add_to_scenegraph_owned()?;
		Spatial::add_to(&node.0, None, Mat4::IDENTITY, false);
		let hand = InputDataType::Hand(Hand {
			right: matches!(side, HandSide::Right),
			..Default::default()
		});
		let datamap = Datamap::from_typed(HandDatamap::default())?;
		let input = InputMethod::add_to(&node.0, hand, datamap)?;

		let material = if transparent {
			// Use holdout material for passthrough
			HandMaterial::Holdout(holdout_materials.add(HandHoldoutMaterial {
				base: BevyMaterial {
					base_color: Srgba::new(0.0, 0.0, 0.0, 1.0).into(),
					base_color_texture: Some(GRADIENT_TEXTURE_HANDLE),
					perceptual_roughness: 1.0,
					..default()
				},
				extension: HandHoldoutExtension {},
			}))
		} else {
			// Normal material
			HandMaterial::Normal(materials.add(BevyMaterial {
				base_color: Srgba::new(1.0, 1.0, 1.0, 1.0).into(),
				alpha_mode: AlphaMode::Blend,
				base_color_texture: Some(GRADIENT_TEXTURE_HANDLE),
				perceptual_roughness: 1.0,
				..default()
			}))
		};
		Ok(OxrHandInput {
			_node: node,
			palm_spatial,
			palm_object,
			side,
			input,
			tracked,
			capture_manager: CaptureManager::default(),
			datamap: Default::default(),
			tracker: None,
			material,
			captured: false,
		})
	}
	pub fn set_enabled(&self, enabled: bool) {
		if let Some(node) = self.input.spatial.node() {
			node.set_enabled(enabled);
		}
		self.tracked.set_tracked(enabled);
	}
	fn update(
		&mut self,
		joints: Option<&openxr::HandJointLocations>,
		materials: &mut ResMut<Assets<BevyMaterial>>,
	) {
		// TODO: use the hand data source ext
		let real_hand = true;
		let input_node = self.input.spatial.node().unwrap();
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
		self.set_enabled(is_tracked);
		if is_tracked {
			// cannot ever crash, is_tracked is only true of joints is some
			let joints = joints.unwrap();
			let new_hand = Hand {
				right: matches!(self.side, HandSide::Right),
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
			};
			self.palm_spatial
				.set_local_transform(Mat4::from_rotation_translation(
					new_hand.palm.rotation.into(),
					new_hand.palm.position.into(),
				));

			self.datamap.pinch_strength = pinch_between(&new_hand.thumb.tip, &new_hand.index.tip);
			// this is how stereokit calculates grab
			self.datamap.grab_strength =
				pinch_between(&new_hand.ring.tip, &new_hand.ring.metacarpal);

			*self.input.data.lock() = InputDataType::Hand(new_hand);
			*self.input.datamap.lock() = Datamap::from_typed(&self.datamap).unwrap();
			
			// Only change colors for normal materials (not holdout)
			if let HandMaterial::Normal(material_handle) = &self.material {
				let captured = self.capture_manager.capture.upgrade().is_some();
				if captured && !self.captured {
					materials.get_mut(material_handle).unwrap().base_color =
						Srgba::rgb(0., 1., 0.75).into();
				} else if self.captured && !captured {
					materials.get_mut(material_handle).unwrap().base_color =
						Srgba::rgb(1., 1.0, 1.0).into();
				}
				self.captured = captured;
			}
		}

		let distance_calculator = |space: &Arc<Spatial>, data: &InputDataType, field: &Field| {
			let InputDataType::Hand(hand) = data else {
				return None;
			};
			let thumb_tip_distance = field.distance(space, hand.thumb.tip.position.into());
			let index_tip_distance = field.distance(space, hand.index.tip.position.into());
			let middle_tip_distance = field.distance(space, hand.middle.tip.position.into());
			let ring_tip_distance = field.distance(space, hand.ring.tip.position.into());

			Some(
				(thumb_tip_distance * 0.3)
					+ (index_tip_distance * 0.4)
					+ (middle_tip_distance * 0.15)
					+ (ring_tip_distance * 0.15),
			)
		};

		self.capture_manager.update_capture(&self.input);
		self.capture_manager
			.set_new_capture(&self.input, distance_calculator);
		self.capture_manager.apply_capture(&self.input);

		if self.capture_manager.capture.upgrade().is_some() {
			return;
		}

		let sorted_handlers = get_sorted_handlers(&self.input, distance_calculator);
		self.input
			.set_handler_order(sorted_handlers.iter().map(|(handler, _)| handler));
	}
}

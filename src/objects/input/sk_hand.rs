use crate::core::client::INTERNAL_CLIENT;
use crate::nodes::OwnedNode;
use crate::nodes::fields::{Field, FieldTrait};
use crate::nodes::input::{Finger, INPUT_HANDLER_REGISTRY, InputDataType, InputHandler, Thumb};
use crate::nodes::{
	Node,
	input::{Hand, InputMethod, Joint},
	spatial::Spatial,
};
use crate::objects::{ObjectHandle, SpatialRef, Tracked};
use crate::{DbusConnection, ObjectRegistryRes, PreFrameWait};
use bevy::prelude::Transform as BevyTransform;
use bevy::prelude::*;
use bevy_mod_openxr::helper_traits::{ToQuat, ToVec3};
use bevy_mod_openxr::resources::OxrFrameState;
use bevy_mod_openxr::session::OxrSession;
use bevy_mod_xr::hands::{HandBone, HandSide, XrHandBoneEntities, XrHandBoneRadius};
use bevy_mod_xr::session::{XrPreDestroySession, XrSessionCreated};
use bevy_mod_xr::spaces::{XrPrimaryReferenceSpace, XrSpaceLocationFlags};
use bevy_sk::hand::GRADIENT_TEXTURE_HANDLE;
use bevy_sk::vr_materials::PbrMaterial;
use color_eyre::eyre::Result;
use glam::{Mat4, Quat, Vec3};
use openxr::{HandJointLocation, SpaceLocationFlags};
use serde::{Deserialize, Serialize};
use stardust_xr::values::Datamap;
use std::sync::Arc;
use stereokit_rust::material::Material;
use stereokit_rust::sk::{DisplayMode, MainThreadToken, Sk};
use stereokit_rust::system::{HandJoint, HandSource, Handed, Input, LinePoint, Lines};
use stereokit_rust::util::Color128;
use zbus::Connection;

use super::{CaptureManager, get_sorted_handlers};

pub struct HandPlugin;
impl Plugin for HandPlugin {
	fn build(&self, app: &mut App) {
		app.add_systems(PreFrameWait, update_hands);
		app.add_systems(XrSessionCreated, create_trackers);
		app.add_systems(XrPreDestroySession, destroy_trackers);
		app.add_systems(PostUpdate, update_hand_material);
		app.add_systems(Startup, setup);
	}
}
fn update_hands(
	mut hands: ResMut<Hands>,
	session: Option<Res<OxrSession>>,
	state: Option<Res<OxrFrameState>>,
	ref_space: Option<Res<XrPrimaryReferenceSpace>>,
	mut materials: ResMut<Assets<PbrMaterial>>,
	mut joint_query: Query<(
		&mut BevyTransform,
		&mut XrSpaceLocationFlags,
		&mut XrHandBoneRadius,
	)>,
	joints_query: Query<&XrHandBoneEntities>,
) {
	let (Some(session), Some(state), Some(ref_space)) = (session, state, ref_space) else {
		tokio::task::spawn({
			let left = hands.left.tracked.clone();
			let right = hands.right.tracked.clone();
			async move {
				left.set_tracked(false);
				right.set_tracked(false);
			}
		});
		return;
	};
	let get_joints = |hand: &mut SkHand| -> Option<openxr::HandJointLocations> {
		let Some(tracker) = hand.tracker.as_ref() else {
			hand.input.spatial.node().unwrap().set_enabled(false);
			let handle = hand.tracked.clone();
			tokio::task::spawn(async move {
				handle.set_tracked(false);
			});
			return None;
		};
		// this won't be correct with pipelined rendering
		session
			.locate_hand_joints(tracker, &ref_space, state.predicted_display_time)
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
			With<MeshMaterial3d<PbrMaterial>>,
			Without<CorrectHandMaterial>,
		),
	>,
	mut cmds: Commands,
	hands: Res<Hands>,
) {
	for (entity, side) in &query {
		let handle = match side {
			HandSide::Left => hands.left.material.clone(),
			HandSide::Right => hands.right.material.clone(),
		};
		cmds.entity(entity)
			.insert(MeshMaterial3d(handle))
			.insert(CorrectHandMaterial);
	}
}

fn setup(
	connection: Res<DbusConnection>,
	mut cmds: Commands,
	mut materials: ResMut<Assets<PbrMaterial>>,
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
		left: SkHand::new(&connection, HandSide::Left, &mut materials).unwrap(),
		right: SkHand::new(&connection, HandSide::Right, &mut materials).unwrap(),
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
	left: SkHand,
	right: SkHand,
}

#[derive(Default, Deserialize, Serialize)]
struct HandDatamap {
	pinch_strength: f32,
	grab_strength: f32,
}

pub struct SkHand {
	_node: OwnedNode,
	palm_spatial: Arc<Spatial>,
	palm_object: ObjectHandle<SpatialRef>,
	side: HandSide,
	input: Arc<InputMethod>,
	capture_manager: CaptureManager,
	datamap: HandDatamap,
	tracked: ObjectHandle<Tracked>,
	tracker: Option<openxr::HandTracker>,
	captured: bool,
	material: Handle<PbrMaterial>,
}
impl SkHand {
	pub fn new(
		connection: &Connection,
		side: HandSide,
		materials: &mut Assets<PbrMaterial>,
	) -> Result<Self> {
		let (palm_spatial, palm_object) = SpatialRef::create(
			connection,
			&("/org/stardustxr/Hand/".to_string()
				+ match side {
					HandSide::Left => "left",
					HandSide::Right => "right",
				} + "/palm"),
		);
		let tracked = Tracked::new(
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

		let material = materials.add(PbrMaterial {
			color: Srgba::new(1.0, 1.0, 1.0, 1.0).into(),
			alpha_mode: AlphaMode::Blend,
			use_stereokit_uvs: false,
			diffuse_texture: Some(GRADIENT_TEXTURE_HANDLE),
			roughness: 1.0,
			..default()
		});
		Ok(SkHand {
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
	fn update(
		&mut self,
		joints: Option<&openxr::HandJointLocations>,
		materials: &mut ResMut<Assets<PbrMaterial>>,
	) {
		// TODO: use the hand data source ext
		let real_hand = true;
		let input_node = self.input.spatial.node().unwrap();
		let is_tracked = real_hand
			&& joints.is_some_and(|v| {
				v.iter().all(|v| {
					v.location_flags.contains(
						SpaceLocationFlags::POSITION_VALID
							| SpaceLocationFlags::POSITION_TRACKED
							| SpaceLocationFlags::ORIENTATION_VALID
							| SpaceLocationFlags::ORIENTATION_TRACKED,
					)
				})
			});
		input_node.set_enabled(is_tracked);
		tokio::task::spawn({
			let handle = self.tracked.clone();
			async move {
				handle.set_tracked(is_tracked);
			}
		});
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
			let captured = self.capture_manager.capture.upgrade().is_some();
			if captured && !self.captured {
				materials.get_mut(&self.material).unwrap().color = Srgba::rgb(0., 1., 0.75).into();
			} else if self.captured && !captured {
				materials.get_mut(&self.material).unwrap().color = Srgba::rgb(1., 1.0, 1.0).into();
			}
			self.captured = captured;
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

fn joint_to_line_point(joint: &Joint, color: Color128) -> LinePoint {
	LinePoint {
		pt: Vec3::from(joint.position).into(),
		thickness: joint.radius * 2.0,
		color: color.into(),
	}
}

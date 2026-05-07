use super::{HandlerTracker, InputMethodBase, QueryHandler};
use crate::nodes::ProxyExt;
use crate::nodes::drawable::model::HoldoutExtension;
use crate::nodes::fields::{Field, FieldTrait};
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
    Point, PointsQuery, PointsQueryHandle, PointsQueryHandler, PointsQueryHandlerHandler,
    SpatialQueryGuard, SpatialQueryInterface as SpatialQueryInterfaceProxy,
};
use stardust_xr_protocol::suis::{
    Chirality, DatamapData, Finger, Hand, InputDataType, InputHandler, InputMethod,
    InputMethodHandler, Joint, SemanticData, SpatialData, Thumb,
};
use stardust_xr_protocol::types::{self, Timestamp, Vec3F};
use std::any::type_name;
use std::collections::HashMap;
use std::fmt::Debug;
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use tokio::sync::RwLock;
use tracing::Instrument;
use zbus::Connection;

// Holdout material for transparent hands (passthrough)
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
        // hands.left.tracked.set_tracked(false);
        // hands.right.tracked.set_tracked(false);
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
            )
        {
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

#[derive(Default, Deserialize, Serialize, Clone, Copy)]
struct HandDatamap {
    pinch_strength: f32,
    grab_strength: f32,
}

enum HandMaterial {
    Normal(Handle<BevyMaterial>),
    Holdout(Handle<HandHoldoutMaterial>),
}

pub struct OxrHandInput {
    palm_spatial: BinderObjectRef<SpatialObject>,
    side: HandSide,
    method: Option<BinderObject<HandInputMethod>>,
    datamap: HandDatamap,
    captured: bool,
    material: HandMaterial,
    was_enabled: bool,
    tracker: HandlerTracker,
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
        let hand = InputDataType::Hand {
            data: Hand {
                chirality: match side {
                    HandSide::Left => Chirality::Left,
                    HandSide::Right => Chirality::Right,
                },
                ..Default::default()
            },
        };

        // TODO: maybe make this dynamic through a dbus api?
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
            datamap: Default::default(),
            material,
            captured: false,
            was_enabled: false,
            method: None,
            tracker: HandlerTracker::default(),
        })
    }
    pub fn set_enabled(&self, enabled: bool) {
        // if let Some(node) = self.input.spatial.node() {
        // 	node.set_enabled(enabled);
        // }
        // self.tracked.set_tracked(enabled);
    }
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
        if let Some(new_hand) = new_hand {
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

            self.datamap.pinch_strength = pinch_between(&new_hand.thumb.tip, &new_hand.index.tip);
            // this is how stereokit calculates grab
            self.datamap.grab_strength =
                pinch_between(&new_hand.ring.tip, &new_hand.ring.metacarpal);

            if let HandMaterial::Normal(material_handle) = &self.material {
                let captured = self
                    .method
                    .as_ref()
                    .is_some_and(|m| m.base.capture.blocking_read().is_some());
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
        let Some(method) = self.method.as_ref() else {
            return;
        };
        let mut handler_order = Vec::new();
        let mut new_handlers = std::collections::HashSet::new();
        if let Some(hand) = new_hand
            && method.base.capture.blocking_read().is_none()
        {
            for entry in method.base.handlers.blocking_read().values() {
                let distance =
                    HandInputMethod::hand_sort_distance(base_space, &entry.field.data, &hand);
                handler_order.push((
                    distance,
                    entry.handler.clone(),
                    entry.spatial.clone(),
                    entry.field.clone(),
                ));
                new_handlers.insert(entry.handler.clone());
            }
        }
        if let Some(_hand) = new_hand
            && let Some(capture) = method.base.capture.blocking_read().clone()
        {
            let handlers = method.base.handlers.blocking_read();
            if let Some(entry) = handlers.values().find(|e| e.handler == capture) {
                handler_order.push((
                    0.0,
                    capture.clone(),
                    entry.spatial.clone(),
                    entry.field.clone(),
                ));
                new_handlers.insert(capture);
            } else {
                drop(handlers);
                method.base.capture.blocking_write().take();
            }
        }
        handler_order.sort_by(|(v1, ..), (v2, ..)| v1.total_cmp(v2));
        let (newly_added_handlers, removed_handlers) = self.tracker.update(new_handlers);
        let captured_handler = method.base.capture.blocking_read().clone();
        let method_arc = method.handler_arc().clone();
        let input_method = InputMethod::from_handler(method);
        let data = self.datamap;
        let timestamp = method
            .base_space
            .instance()
            .xr_to_timestamp(time)
            .unwrap_or_else(Timestamp::now);
        let hand_space = base_space.clone();
        tokio::spawn(async move {
            for (i, (_, handler, handler_spatial, handler_field)) in
                handler_order.into_iter().enumerate()
            {
                method_arc.base.maybe_promote_capture(&handler).await;
                // TODO: optimize and cache this
                let mut grab_bindings = std::collections::HashSet::new();
                let mut pinch_bindings = std::collections::HashSet::new();
                let bindings_span = info_span!("suggested-bindings");
                let Ok(bindings) = handler.suggested_bindings().instrument(bindings_span).await
                else {
                    continue;
                };
                for (name, bindings) in bindings {
                    for binding in bindings {
                        if binding == "pinch_strength" || binding == "pinch" {
                            pinch_bindings.insert(name.clone());
                        }
                        if binding == "grab_strength" || binding == "grab" {
                            grab_bindings.insert(name.clone());
                        }
                    }
                }
                let mut datamap = HashMap::new();
                datamap.insert(
                    "pinch_strength".to_string(),
                    DatamapData::Float {
                        value: data.pinch_strength,
                    },
                );
                datamap.insert(
                    "grab_strength".to_string(),
                    DatamapData::Float {
                        value: data.grab_strength,
                    },
                );
                for binding in grab_bindings {
                    if let DatamapData::Float { value } = datamap
                        .entry(binding)
                        .or_insert(DatamapData::Float { value: 0.0 })
                    {
                        *value = data.grab_strength.max(*value);
                    }
                }
                for binding in pinch_bindings {
                    if let DatamapData::Float { value } = datamap
                        .entry(binding)
                        .or_insert(DatamapData::Float { value: 0.0 })
                    {
                        *value = data.pinch_strength.max(*value);
                    }
                }
                let hand = new_hand.unwrap();
                let distance =
                    HandInputMethod::hand_real_distance(&hand_space, &handler_field.data, &hand);
                let hand = HandInputMethod::localize_hand(
                    &hand_space,
                    &hand,
                    &handler_spatial,
                    &handler_field.data,
                );
                let spatial_data = SpatialData {
                    input: InputDataType::Hand { data: hand },
                    distance,
                };
                let semantic_data = SemanticData {
                    datamap,
                    order: i as u32,
                    captured: captured_handler.as_ref().is_some_and(|v| v == &handler),
                };
                if newly_added_handlers.contains(&handler) {
                    let _span = info_span!("input-gained").entered();
                    handler.input_gained(
                        input_method.clone(),
                        timestamp,
                        spatial_data,
                        semantic_data,
                    );
                } else {
                    let _span = info_span!("input-updated").entered();
                    handler.input_updated(
                        input_method.clone(),
                        timestamp,
                        spatial_data,
                        semantic_data,
                    );
                }
            }
            for handler in removed_handlers {
                let _span = info_span!("input-left").entered();
                handler.input_left(input_method.clone(), timestamp);
            }
        });
        self.captured = method.base.capture.blocking_read().is_some();
    }
}

#[derive(Debug, Handler)]
struct HandInputMethod {
    side: HandSide,
    base_space: DebugWrapper<Arc<openxr::Space>>,
    base_spatial: BinderObjectRef<SpatialRef>,
    tracker: DebugWrapper<openxr::HandTracker>,
    base: InputMethodBase<()>,
    query: BinderObject<InputHandlerQuery>,
    query_handle: Arc<OnceLock<PointsQueryHandle>>,
}
impl HandInputMethod {
    fn new(
        base_spatial: BinderObjectRef<SpatialRef>,
        base_space: Arc<openxr::Space>,
        side: HandSide,
        tracker: openxr::HandTracker,
    ) -> Result<Self, GluonSendError> {
        let (query_handler, handlers) = QueryHandler::new();
        let query = PION.register_object(InputHandlerQuery {
            query: query_handler,
        });
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
            base: InputMethodBase::new(handlers),
            query,
            query_handle,
        })
    }
    fn locate_hand(
        &self,
        relative_to: &BinderObjectRef<SpatialRef>,
        time: openxr::Time,
    ) -> Option<Hand> {
        let joints = {
            let mat = Spatial::space_to_space_matrix(Some(&self.base_spatial), (Some(relative_to)));
            self.base_space
                .locate_hand_joints(&self.tracker, time)
                .inspect_err(|err| error!("Error while locating hand joints: {err}"))
                .ok()
                .flatten()
                .map(|joints| {
                    joints.map(|mut j| {
                        // TODO: scale joint radius?
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
        // TODO: use the hand data source ext
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
            // cannot ever crash, is_tracked is only true of joints is some
            let joints = joints.unwrap();
            let new_hand = Hand {
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
            };
            Some(new_hand)
        } else {
            None
        }
    }
    pub fn hand_sort_distance(
        hand_space: &BinderObjectRef<SpatialRef>,
        field: &Field,
        hand: &Hand,
    ) -> f32 {
        let thumb_tip_distance = field.distance(hand_space, hand.thumb.tip.pose.position.mint());
        let index_tip_distance = field.distance(hand_space, hand.index.tip.pose.position.mint());
        let middle_tip_distance =
            field.distance(hand_space, hand.middle.tip.pose.position.mint());
        let ring_tip_distance = field.distance(hand_space, hand.ring.tip.pose.position.mint());

        (thumb_tip_distance * 0.3)
            + (index_tip_distance * 0.4)
            + (middle_tip_distance * 0.15)
            + (ring_tip_distance * 0.15)
    }
    pub fn hand_real_distance(
        hand_space: &BinderObjectRef<SpatialRef>,
        field: &Field,
        hand: &Hand,
    ) -> f32 {
        let get_dist =
            |joint: &Joint| field.distance(hand_space, joint.pose.position.mint()) - joint.radius;

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
        // TODO: apply scaling
        radius: joint.radius,
        distance: field.distance(from, joint.pose.position.mint()),
    }
}

impl InputMethodHandler for HandInputMethod {
    async fn request_capture(&self, _ctx: gluon_wire::GluonCtx, handler: InputHandler) {
        self.base.request_capture(handler).await;
    }

    async fn release_capture(&self, _ctx: gluon_wire::GluonCtx, handler: InputHandler) {
        self.base.release_capture(&handler).await;
    }

    async fn get_spatial_data(
        &self,
        _ctx: gluon_wire::GluonCtx,
        handler: InputHandler,
        time: Timestamp,
    ) -> Option<SpatialData> {
        let time = self.base_space.instance().timestamp_to_xr(time)?;
        let (spatial, field) = {
            let handlers = self.base.handlers.read().await;
            let entry = handlers.values().find(|e| e.handler == handler)?;
            (entry.spatial.clone(), entry.field.clone())
        };
        let hand = self.locate_hand(&spatial, time)?;
        let distance = Self::hand_real_distance(&spatial, &field.data, &hand);
        Some(SpatialData {
            input: InputDataType::Hand { data: hand },
            distance,
        })
    }
}

#[derive(Debug, Handler)]
struct InputHandlerQuery {
    query: QueryHandler<()>,
}

impl PointsQueryHandlerHandler for InputHandlerQuery {
    async fn entered(
        &self,
        _ctx: gluon_wire::GluonCtx,
        obj: QueryableObjectRef,
        field: stardust_xr_protocol::field::FieldRef,
        spatial: stardust_xr_protocol::spatial::SpatialRef,
        interfaces: Vec<QueriedInterface>,
        _distance: f32,
    ) {
        self.query.on_entered(obj, field, spatial, interfaces, ()).await;
    }

    async fn interfaces_changed(
        &self,
        _ctx: gluon_wire::GluonCtx,
        _obj: QueryableObjectRef,
        _interfaces: Vec<QueriedInterface>,
    ) {
    }

    async fn moved(&self, _ctx: gluon_wire::GluonCtx, _obj: QueryableObjectRef, _distance: f32) {}

    async fn left(&self, _ctx: gluon_wire::GluonCtx, obj: QueryableObjectRef) {
        self.query.on_left(&obj).await;
    }
}

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

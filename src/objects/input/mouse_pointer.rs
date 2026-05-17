use super::{BeamQueryCache, BeamValue, CachedObject, InputSender, InputSource, QueryCache};
use crate::{
	PION,
	bevy_int::flatscreen_cam::FlatscreenCam,
	nodes::{
		fields::{Field, Ray},
		spatial::{Spatial, SpatialObject, SpatialRef},
	},
	query::spatial_query::SpatialQueryInterface,
};
use bevy::{input::mouse::MouseWheel, prelude::*, window::PrimaryWindow};
use color_eyre::eyre::Result;
use glam::{Mat4, Vec3};
use gluon::{Handler, Object};
use mint::Vector2;
use stardust_xr_protocol::{
	field::FieldRef as FieldRefProxy,
	query::{InterfaceDependency, QueryableObjectRef},
	spatial::SpatialRef as SpatialRefProxy,
	spatial_query::{
		BeamQuery, BeamQueryHandler, SpatialQueryGuard,
		SpatialQueryInterface as SpatialQueryInterfaceProxy,
	},
	suis::{
		DatamapData, InputDataType, InputHandler, InputMethod, InputMethodHandler, Pointer,
		SpatialData,
	},
	types::{Timestamp, Vec3F},
};
use std::{
	collections::{HashMap, HashSet},
	sync::{Arc, OnceLock},
};
use tokio::sync::RwLock;

pub struct FlatscreenInputPlugin;
impl Plugin for FlatscreenInputPlugin {
	fn build(&self, app: &mut App) {
		app.add_systems(Startup, setup);
		app.add_systems(Update, update_pointer);
	}
}

fn setup(mut cmds: Commands) {
	let Ok(pointer) =
		MousePointer::new().inspect_err(|err| error!("unable to create mouse pointer: {err}"))
	else {
		return;
	};
	cmds.insert_resource(pointer);
}

fn update_pointer(
	window: Single<&Window, With<PrimaryWindow>>,
	cam: Single<(&Camera, &GlobalTransform), With<FlatscreenCam>>,
	mut pointer: ResMut<MousePointer>,
	mouse_buttons: Res<ButtonInput<MouseButton>>,
	keyboard_buttons: Res<ButtonInput<KeyCode>>,
	scroll: EventReader<MouseWheel>,
) {
	if keyboard_buttons.pressed(KeyCode::ShiftLeft) && mouse_buttons.pressed(MouseButton::Right) {
		return;
	}

	let (cam, cam_transform) = *cam;
	let Some(ray) = window
		.cursor_position()
		.and_then(|pos| get_viewport_pos(pos, cam))
		.and_then(|pos| cam.viewport_to_world(cam_transform, pos).ok())
	else {
		return;
	};
	pointer.update(ray, &mouse_buttons, scroll);
}

fn get_viewport_pos(logical_pos: Vec2, cam: &Camera) -> Option<Vec2> {
	if let Some(viewport_rect) = cam.logical_viewport_rect() {
		if !viewport_rect.contains(logical_pos) {
			return None;
		}
		Some(logical_pos - viewport_rect.min)
	} else {
		Some(logical_pos)
	}
}

#[derive(Debug, Clone, Copy)]
struct MouseEvent {
	select: f32,
	middle: f32,
	context: f32,
	grab: f32,
	scroll_continuous: Vector2<f32>,
	scroll_discrete: Vector2<f32>,
}
impl Default for MouseEvent {
	fn default() -> Self {
		MouseEvent {
			select: 0.0,
			middle: 0.0,
			context: 0.0,
			grab: 0.0,
			scroll_continuous: [0.0; 2].into(),
			scroll_discrete: [0.0; 2].into(),
		}
	}
}

// ── MouseMethod ───────────────────────────────────────────────────────────────

#[derive(Debug, Handler)]
struct MouseMethod {
	spatial_arc: Arc<Spatial>,
	event: RwLock<MouseEvent>,
	capture: RwLock<Option<InputHandler>>,
	sender: Arc<InputSender<BeamValue>>,
	_beam_query: Object<BeamQueryCache>,
	_query_guard: Arc<OnceLock<SpatialQueryGuard>>,
}

impl InputSource for MouseMethod {
	type QueryValue = BeamValue;

	fn order_handlers_and_captures(
		&self,
		objects: &HashMap<QueryableObjectRef, CachedObject<Self::QueryValue>>,
		capture_requests: &HashSet<InputHandler>,
	) -> (Vec<InputHandler>, Option<InputHandler>) {
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

		let mut order: Vec<_> = if let Some(ref cap) = capture {
			objects
				.values()
				.filter(|e| &e.handler == cap)
				.map(|e| (e.value.deepest_point_distance, e.handler.clone()))
				.collect()
		} else {
			objects
				.values()
				.map(|e| (e.value.deepest_point_distance, e.handler.clone()))
				.collect()
		};
		order.sort_by(|(d1, _), (d2, _)| d1.total_cmp(d2));

		(order.into_iter().map(|(_, h)| h).collect(), capture)
	}

	fn spatial_data(&self, handler_spatial: &SpatialRef, handler_field: &Field) -> SpatialData {
		let ray_result = handler_field.ray_march(Ray {
			origin: Vec3::ZERO,
			direction: Vec3::NEG_Z,
			space: self.spatial_arc.clone(),
		});
		let ptr_to_handler =
			Spatial::space_to_space_matrix(Some(&*self.spatial_arc), Some(handler_spatial));
		let (_, rotation, translation) = ptr_to_handler.to_scale_rotation_translation();
		SpatialData {
			input: InputDataType::Pointer {
				data: Pointer {
					pose: stardust_xr_protocol::types::Posef {
						position: translation.into(),
						orientation: rotation.into(),
					},
					deepest_point: ray_result.deepest_point_distance,
				},
			},
			distance: ray_result.min_distance,
		}
	}

	fn datamap(
		&self,
		_suggested_bindings: &HashMap<String, Vec<String>>,
	) -> HashMap<String, DatamapData> {
		let event = *self.event.blocking_read();
		build_datamap(&event)
	}
}

impl InputMethodHandler for MouseMethod {
	async fn request_capture(&self, _ctx: gluon::Context, handler: InputHandler) {
		self.sender.request_capture(handler).await;
	}

	async fn release_capture(&self, _ctx: gluon::Context, handler: InputHandler) {
		self.sender.release_capture(&handler).await;
		let mut cap = self.capture.write().await;
		if cap.as_ref() == Some(&handler) {
			cap.take();
		}
	}

	async fn get_spatial_data(
		&self,
		_ctx: gluon::Context,
		handler: InputHandler,
		_time: Timestamp,
	) -> Option<SpatialData> {
		let cap = self.capture.read().await.clone();
		if cap.as_ref().is_some_and(|c| c != &handler) {
			return None;
		}
		let objects = self.sender.cache.read().await;
		let entry = objects.values().find(|e| e.handler == handler)?;
		Some(self.spatial_data(&entry.spatial, &entry.field.data))
	}
}

// ── MousePointer ──────────────────────────────────────────────────────────────

#[derive(Resource)]
pub struct MousePointer {
	spatial: gluon::ObjectRef<SpatialObject>,
	method: gluon::Object<MouseMethod>,
}

impl MousePointer {
	pub fn new() -> Result<Self> {
		let spatial = SpatialObject::new(None, Mat4::IDENTITY);
		let spatial_arc = (**spatial).clone();

		let (query_cache, objects_arc) = QueryCache::new();
		let sender = Arc::new(InputSender::new(objects_arc));

		let beam_query = PION.register_object(BeamQueryCache(query_cache));
		let beam_handler_proxy = BeamQueryHandler::from_handler(&beam_query);

		let query_guard: Arc<OnceLock<SpatialQueryGuard>> = Arc::new(OnceLock::new());
		let base_spatial_ref = SpatialRefProxy::from_handler(spatial.get_ref());
		tokio::spawn({
			let query_guard = query_guard.clone();
			async move {
				let sqi = SpatialQueryInterface::new(&Arc::default());
				let sqi_proxy = SpatialQueryInterfaceProxy::from_handler(&sqi);
				match sqi_proxy
					.beam_query(BeamQuery {
						handler: beam_handler_proxy,
						interfaces: vec![InterfaceDependency {
							id: "org.stardustxr.SUIS.Handler".to_string(),
							optional: false,
						}],
						reference_spatial: base_spatial_ref,
						origin: Vec3F {
							x: 0.0,
							y: 0.0,
							z: 0.0,
						},
						direction: Vec3F {
							x: 0.0,
							y: 0.0,
							z: -1.0,
						},
						max_length: f32::MAX,
					})
					.await
				{
					Ok(guard) => {
						query_guard.set(guard).ok();
					}
					Err(e) => {
						error!("failed to create mouse pointer beam query: {e}");
					}
				}
			}
		});

		let method = PION.register_object(MouseMethod {
			spatial_arc,
			event: RwLock::new(MouseEvent::default()),
			capture: RwLock::new(None),
			sender,
			_beam_query: beam_query,
			_query_guard: query_guard,
		});

		Ok(MousePointer { spatial, method })
	}

	pub fn update(
		&mut self,
		ray: Ray3d,
		mouse_buttons: &ButtonInput<MouseButton>,
		mut scroll: EventReader<MouseWheel>,
	) {
		let mut discrete = Vec2::ZERO;
		let mut continuous = Vec2::ZERO;
		for e in scroll.read() {
			match e.unit {
				bevy::input::mouse::MouseScrollUnit::Line => {
					discrete.x += e.x;
					discrete.y -= e.y;
				}
				bevy::input::mouse::MouseScrollUnit::Pixel => {
					continuous.x += e.x;
					continuous.y -= e.y;
				}
			}
		}

		self.spatial.set_local_transform(
			Mat4::look_to_rh(ray.origin, Vec3::from(ray.direction), Vec3::Y).inverse(),
		);

		*self.method.event.blocking_write() = MouseEvent {
			select: mouse_buttons.pressed(MouseButton::Left) as u32 as f32,
			middle: mouse_buttons.pressed(MouseButton::Middle) as u32 as f32,
			context: mouse_buttons.pressed(MouseButton::Right) as u32 as f32,
			grab: mouse_buttons.pressed(MouseButton::Right) as u32 as f32,
			scroll_continuous: continuous.into(),
			scroll_discrete: discrete.into(),
		};

		let input_method = InputMethod::from_handler(&self.method);
		let sender = self.method.sender.clone();
		sender.send(&**self.method, input_method, Timestamp::now());
	}
}

fn build_datamap(event: &MouseEvent) -> HashMap<String, DatamapData> {
	let mut map = HashMap::new();
	map.insert(
		"select".to_string(),
		DatamapData::Float {
			value: event.select,
		},
	);
	map.insert(
		"middle".to_string(),
		DatamapData::Float {
			value: event.middle,
		},
	);
	map.insert(
		"context".to_string(),
		DatamapData::Float {
			value: event.context,
		},
	);
	map.insert("grab".to_string(), DatamapData::Float { value: event.grab });
	map.insert(
		"scroll_continuous_x".to_string(),
		DatamapData::Float {
			value: event.scroll_continuous.x,
		},
	);
	map.insert(
		"scroll_continuous_y".to_string(),
		DatamapData::Float {
			value: event.scroll_continuous.y,
		},
	);
	map.insert(
		"scroll_discrete_x".to_string(),
		DatamapData::Float {
			value: event.scroll_discrete.x,
		},
	);
	map.insert(
		"scroll_discrete_y".to_string(),
		DatamapData::Float {
			value: event.scroll_discrete.y,
		},
	);
	map
}

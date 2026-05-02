use crate::{
	PION,
	bevy_int::flatscreen_cam::FlatscreenCam,
	nodes::{
		ProxyExt as _,
		fields::{FieldTrait, Ray},
		spatial::{Spatial, SpatialObject},
	},
	query::spatial_query::SpatialQueryInterface,
};
use bevy::{input::mouse::MouseWheel, prelude::*, window::PrimaryWindow};
use binderbinder::binder_object::{BinderObject, BinderObjectRef, ToBinderObjectOrRef};
use color_eyre::eyre::Result;
use glam::{Mat4, Vec3};
use gluon_wire::impl_transaction_handler;
use mint::Vector2;
use stardust_xr_protocol::{
	field::FieldRef as FieldRefProto,
	input::{
		DatamapData, InputData, InputDataType, InputHandler, InputMethod, InputMethodHandler,
		Pointer, SpatialInputData,
	},
	query::{InterfaceDependency, QueriedInterface, QueryableObjectRef},
	spatial::SpatialRef as SpatialRefProxy,
	spatial_query::{
		BeamQuery, BeamQueryHandler, BeamQueryHandlerHandler, SpatialQueryGuard,
		SpatialQueryInterface as SpatialQueryInterfaceProxy,
	},
	types::{Timestamp, Vec3F},
};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};
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
	// Don't deliver pointer input while the fly camera is active.
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

#[derive(Debug)]
struct MouseInputMethod {
	_beam_handler: BinderObject<MouseBeamHandler>,
	capture: RwLock<Option<InputHandler>>,
	capture_requests: RwLock<HashSet<InputHandler>>,
	queried_handlers: Arc<RwLock<HashMap<InputHandler, f32>>>,
	spatial_arc: Arc<Spatial>,
}

impl InputMethodHandler for MouseInputMethod {
	async fn request_capture(&self, _ctx: gluon_wire::GluonCtx, handler: InputHandler) {
		if self.queried_handlers.read().await.contains_key(&handler) {
			self.capture_requests.write().await.insert(handler);
		}
	}

	async fn release_capture(&self, _ctx: gluon_wire::GluonCtx, handler: InputHandler) {
		self.capture_requests.write().await.remove(&handler);
		if self
			.capture
			.read()
			.await
			.as_ref()
			.is_some_and(|h| h == &handler)
		{
			self.capture.write().await.take();
		}
	}

	async fn get_spatial_data(
		&self,
		_ctx: gluon_wire::GluonCtx,
		handler: InputHandler,
		_time: Timestamp,
	) -> Option<SpatialInputData> {
		let handler_space = handler
			.get_spatial()
			.await
			.inspect_err(|e| error!("failed to get spatial for input handler: {e}"))
			.ok()?
			.owned()?;
		info!("got space: {handler_space:?}");
		let handler_field = handler
			.get_field()
			.await
			.inspect_err(|e| error!("failed to get field for input handler: {e}"))
			.ok()?
			.owned()?;

		info!("got field: {handler_field:?}");

		let ray_result = handler_field.data.ray_march(Ray {
			origin: Vec3::ZERO,
			direction: Vec3::NEG_Z,
			space: self.spatial_arc.clone(),
		});

		// Transform pointer origin and orientation into the handler's coordinate space.
		let ptr_to_handler =
			Spatial::space_to_space_matrix(Some(&*self.spatial_arc), Some(&***handler_space));
		let (_, rotation, translation) = ptr_to_handler.to_scale_rotation_translation();

		Some(SpatialInputData {
			input: InputDataType::Pointer {
				data: Pointer {
					origin: translation.into(),
					orientation: rotation.into(),
					deepest_point: ray_result.deepest_point_distance,
				},
			},
			distance: ray_result.min_distance,
		})
	}
}

impl_transaction_handler!(MouseInputMethod);

#[derive(Debug)]
struct MouseBeamHandler {
	queried_handlers: Arc<RwLock<HashMap<InputHandler, f32>>>,
	queried_objects: RwLock<HashMap<QueryableObjectRef, InputHandler>>,
}

impl BeamQueryHandlerHandler for MouseBeamHandler {
	async fn intersected(
		&self,
		_ctx: gluon_wire::GluonCtx,
		obj: QueryableObjectRef,
		_field: FieldRefProto,
		_spatial: SpatialRefProxy,
		interfaces: Vec<QueriedInterface>,
		deepest_point_distance: f32,
		_distance: f32,
	) {
		let Some(interface) = interfaces.first() else {
			return;
		};
		if interface.interface_id != "org.stardustxr.SUIS.Handler" {
			return;
		}
		let handler = InputHandler::from_object_or_ref(interface.interface.clone());
		self.queried_objects
			.write()
			.await
			.insert(obj, handler.clone());
		self.queried_handlers
			.write()
			.await
			.insert(handler, deepest_point_distance);
	}

	async fn interfaces_changed(
		&self,
		_ctx: gluon_wire::GluonCtx,
		_obj: QueryableObjectRef,
		_interfaces: Vec<QueriedInterface>,
	) {
	}

	async fn moved(
		&self,
		_ctx: gluon_wire::GluonCtx,
		obj: QueryableObjectRef,
		deepest_point_distance: f32,
		_distance: f32,
	) {
		let objects = self.queried_objects.read().await;
		if let Some(handler) = objects.get(&obj) {
			if let Some(entry) = self.queried_handlers.write().await.get_mut(handler) {
				*entry = deepest_point_distance;
			}
		}
	}

	async fn left(&self, _ctx: gluon_wire::GluonCtx, obj: QueryableObjectRef) {
		if let Some(handler) = self.queried_objects.write().await.remove(&obj) {
			self.queried_handlers.write().await.remove(&handler);
		}
	}
}

impl_transaction_handler!(MouseBeamHandler);

#[derive(Resource)]
pub struct MousePointer {
	spatial: BinderObjectRef<SpatialObject>,
	method: BinderObject<MouseInputMethod>,
	active_handlers: HashSet<InputHandler>,
	_query_guard: Arc<OnceLock<SpatialQueryGuard>>,
	mouse_datamap: MouseEvent,
}

impl MousePointer {
	pub fn new() -> Result<Self> {
		let spatial = SpatialObject::new(None, Mat4::IDENTITY);
		let spatial_arc = (**spatial).clone();
		let queried_handlers: Arc<RwLock<HashMap<InputHandler, f32>>> =
			Arc::new(RwLock::new(HashMap::new()));

		let beam_handler = PION.register_object(MouseBeamHandler {
			queried_handlers: queried_handlers.clone(),
			queried_objects: RwLock::new(HashMap::new()),
		});
		let beam_handler_proxy = BeamQueryHandler::from_handler(&beam_handler);

		let method = PION.register_object(MouseInputMethod {
			_beam_handler: beam_handler,
			capture: RwLock::new(None),
			capture_requests: RwLock::new(HashSet::new()),
			queried_handlers,
			spatial_arc,
		});

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

		Ok(MousePointer {
			spatial,
			method,
			active_handlers: HashSet::new(),
			_query_guard: query_guard,
			mouse_datamap: Default::default(),
		})
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

		self.mouse_datamap = MouseEvent {
			select: mouse_buttons.pressed(MouseButton::Left) as u32 as f32,
			middle: mouse_buttons.pressed(MouseButton::Middle) as u32 as f32,
			context: mouse_buttons.pressed(MouseButton::Right) as u32 as f32,
			grab: mouse_buttons.pressed(MouseButton::Right) as u32 as f32,
			scroll_continuous: continuous.into(),
			scroll_discrete: discrete.into(),
		};

		self.deliver_input();
	}

	fn deliver_input(&mut self) {
		let queried = self.method.queried_handlers.blocking_read();
		let captured = self.method.capture.blocking_read().clone();

		let mut handler_order: Vec<(f32, InputHandler)> =
			queried.iter().map(|(h, &dist)| (dist, h.clone())).collect();
		drop(queried);
		handler_order.sort_by(|(d1, _), (d2, _)| d1.total_cmp(d2));

		let new_handlers: HashSet<InputHandler> =
			handler_order.iter().map(|(_, h)| h.clone()).collect();

		let mut newly_added = HashSet::new();
		for h in &new_handlers {
			if !self.active_handlers.contains(h) {
				newly_added.insert(h.clone());
			}
		}
		let mut removed = HashSet::new();
		for h in &self.active_handlers {
			if !new_handlers.contains(h) {
				removed.insert(h.clone());
			}
		}

		let mouse_datamap = self.mouse_datamap;
		let method_arc = self.method.handler_arc().clone();
		let input_method = InputMethod::from_handler(&self.method);

		tokio::spawn(async move {
			for (i, (_, handler)) in handler_order.into_iter().enumerate() {
				if method_arc.capture_requests.read().await.contains(&handler)
					&& method_arc.capture.read().await.is_none()
				{
					method_arc.capture.write().await.replace(handler.clone());
				}

				let is_captured = captured.as_ref().is_some_and(|c| c == &handler);
				let input_data = InputData {
					datamap: build_datamap(&mouse_datamap),
					order: i as u32,
					captured: is_captured,
				};

				if newly_added.contains(&handler) {
					handler.input_gained(input_method.clone(), input_data);
				} else {
					handler.input_updated(input_method.clone(), input_data);
				}
			}
			for handler in removed {
				handler.input_left(input_method.clone());
			}
		});

		self.active_handlers = new_handlers;
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

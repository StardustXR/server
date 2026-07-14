use super::{BeamQueryCache, BeamValue, CachedObject, InputSender, InputSource, QueryCache};
use crate::{
	PION,
	bevy_int::flatscreen_cam::FlatscreenCam,
	keymap_store::KEYMAP_STORE,
	nodes::{
		fields::{Field, Ray},
		spatial::{Spatial, SpatialObject, SpatialRef},
	},
	query::spatial_query::SpatialQueryInterface,
};
use bevy::{
	input::{
		keyboard::{KeyboardInput, NativeKeyCode},
		mouse::MouseWheel,
	},
	prelude::*,
	window::PrimaryWindow,
};
use color_eyre::eyre::{Result, eyre};
use glam::{Mat4, Vec3};
use gluon::{Handler, Object};
use mint::Vector2;
use stardust_xr_molecules_protocols::keyboard_handler::{
	EXTERNAL_PROTOCOL as KEYBOARD_PROTOCOL, KeyEvent, KeyboardHandler as KeyboardHandlerProxy,
	ModifierState,
};
use stardust_xr_protocol::{
	field::FieldRef as FieldRefProxy,
	keymap::Keymap as KeymapProxy,
	query::{InterfaceDependency, QueriedInterface, QueryableObjectRef},
	spatial::SpatialRef as SpatialRefProxy,
	spatial_query::{
		BeamQuery, BeamQueryHandler, Point, PointsQuery,
		PointsQueryHandle as PointsQueryHandleProxy, PointsQueryHandler, PointsQueryHandlerHandler,
		SpatialQueryGuard, SpatialQueryInterface as SpatialQueryInterfaceProxy,
	},
	suis::{
		DatamapData, InputDataType, InputHandler, InputMethod, InputMethodCapture,
		InputMethodHandler, Pointer, SpatialData,
	},
	types::{Timestamp, Vec3F},
};
use std::{
	collections::{HashMap, HashSet},
	sync::{Arc, Mutex, OnceLock},
};
use tokio::sync::RwLock;
use tracing::instrument;
use xkbcommon_rs::{
	Context as XkbContext, Keymap as XkbKeymap, KeymapFormat, State as XkbState,
	xkb_context::ContextFlags,
	xkb_keymap::CompileFlags,
	xkb_state::{KeyDirection, StateComponent},
};

/// How close (in meters) a keyboard handler's field must be to the pointer's hit
/// point to receive keyboard focus.
const KEYBOARD_FOCUS_MARGIN: f32 = 0.05;

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
	mut key_events: EventReader<KeyboardInput>,
) {
	if keyboard_buttons.pressed(KeyCode::ShiftLeft) && mouse_buttons.pressed(MouseButton::Right) {
		key_events.clear();
		return;
	}

	let (cam, cam_transform) = *cam;
	let Some(ray) = window
		.cursor_position()
		.and_then(|pos| get_viewport_pos(pos, cam))
		.and_then(|pos| cam.viewport_to_world(cam_transform, pos).ok())
	else {
		key_events.clear();
		return;
	};
	pointer.update(ray, &mouse_buttons, scroll, key_events);
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

// ── Keyboard focus ────────────────────────────────────────────────────────────

/// Tracks keyboard handlers matched by the points query anchored at the pointer's
/// current hit point. The closest one gets the key events.
#[derive(Debug, Default, Handler)]
struct KeyboardQueryCache {
	handlers: Mutex<HashMap<QueryableObjectRef, (KeyboardHandlerProxy, f32)>>,
}

impl KeyboardQueryCache {
	fn closest(&self) -> Option<KeyboardHandlerProxy> {
		self.handlers
			.lock()
			.unwrap()
			.values()
			.min_by(|(_, d1), (_, d2)| d1.total_cmp(d2))
			.map(|(handler, _)| handler.clone())
	}
}

impl PointsQueryHandlerHandler for KeyboardQueryCache {
	async fn entered(
		&self,
		_ctx: gluon::Context,
		obj: QueryableObjectRef,
		_field: FieldRefProxy,
		_spatial: SpatialRefProxy,
		interfaces: Vec<QueriedInterface>,
		distance: f32,
	) {
		let Some(interface) = interfaces.first() else {
			return;
		};
		if interface.interface_id != KEYBOARD_PROTOCOL.protocol_name {
			return;
		}
		let handler = KeyboardHandlerProxy::from_object_or_ref(interface.interface.clone());
		self.handlers
			.lock()
			.unwrap()
			.insert(obj, (handler, distance));
	}

	async fn interfaces_changed(
		&self,
		_ctx: gluon::Context,
		_obj: QueryableObjectRef,
		_interfaces: Vec<QueriedInterface>,
	) {
	}

	async fn moved(&self, _ctx: gluon::Context, obj: QueryableObjectRef, distance: f32) {
		if let Some(entry) = self.handlers.lock().unwrap().get_mut(&obj) {
			entry.1 = distance;
		}
	}

	async fn left(&self, _ctx: gluon::Context, obj: QueryableObjectRef) {
		self.handlers.lock().unwrap().remove(&obj);
	}
}

/// Everything needed to turn bevy key events into `KeyboardHandler.key` calls:
/// the query cache above, the query's handle (to move the focus point to the
/// pointer's hit each frame), and xkb state for modifiers + the keymap token.
struct KeyboardFocus {
	cache: Object<KeyboardQueryCache>,
	points_handle: Arc<OnceLock<PointsQueryHandleProxy>>,
	xkb_state: XkbState,
	keymap_string: String,
	keymap_proxy: OnceLock<KeymapProxy>,
}

impl KeyboardFocus {
	/// Lazily register our default keymap with the keymap store; the store may not
	/// be exposed yet when the pointer is created.
	fn keymap_proxy(&self) -> Option<KeymapProxy> {
		if let Some(proxy) = self.keymap_proxy.get() {
			return Some(proxy.clone());
		}
		let store = KEYMAP_STORE.get()?;
		let mut bytes = self.keymap_string.clone().into_bytes();
		bytes.push(0);
		let proxy = store
			.register_keymap_bytes(&bytes)
			.inspect_err(|err| error!("failed to register mouse pointer keymap: {err:?}"))
			.ok()?;
		_ = self.keymap_proxy.set(proxy.clone());
		Some(proxy)
	}
}

// ── MouseMethod ───────────────────────────────────────────────────────────────

#[derive(Debug, Handler)]
struct MouseMethod {
	spatial_arc: Arc<Spatial>,
	event: RwLock<MouseEvent>,
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
		let current_capture = self.sender.active_capture.blocking_read().clone();

		let capture = if let Some(cap) = current_capture {
			if objects.values().any(|e| e.handler == cap) {
				Some(cap)
			} else {
				self.sender.active_capture.blocking_write().take();
				None
			}
		} else {
			let promoted = capture_requests
				.iter()
				.find(|r| objects.values().any(|e| &e.handler == *r))
				.cloned();
			if let Some(ref p) = promoted {
				*self.sender.active_capture.blocking_write() = Some(p.clone());
			}
			promoted
		};

		let mut order: Vec<_> = if let Some(ref cap) = capture {
			objects
				.values()
				.filter(|e| e.spatial.is_some() && &e.handler == cap)
				.map(|e| (e.value.deepest_point_distance, e.handler.clone()))
				.collect()
		} else {
			objects
				.values()
				.filter(|e| e.spatial.is_some())
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

	fn datamap(&self) -> HashMap<String, DatamapData> {
		let event = *self.event.blocking_read();
		build_datamap(&event)
	}
}

impl InputMethodHandler for MouseMethod {
	async fn request_capture(
		&self,
		_ctx: gluon::Context,
		handler: InputHandler,
	) -> Option<InputMethodCapture> {
		self.sender.grant_capture(handler).await
	}

	async fn get_spatial_data(
		&self,
		_ctx: gluon::Context,
		handler: InputHandler,
		_time: Timestamp,
	) -> Option<SpatialData> {
		let cap = self.sender.active_capture.read().await.clone();
		if cap.as_ref().is_some_and(|c| c != &handler) {
			return None;
		}
		let objects = self.sender.cache.read().await;
		let entry = objects.values().find(|e| e.handler == handler)?;
		Some(self.spatial_data(entry.spatial.as_deref()?, &entry.field.data))
	}
}

// ── MousePointer ──────────────────────────────────────────────────────────────

#[derive(Resource)]
pub struct MousePointer {
	spatial: gluon::ObjectRef<SpatialObject>,
	method: gluon::Object<MouseMethod>,
	keyboard: KeyboardFocus,
}

impl MousePointer {
	pub fn new() -> Result<Self> {
		let spatial = SpatialObject::new(None, Mat4::IDENTITY);
		let spatial_arc = (**spatial).clone();

		let (query_cache, objects_arc, capture_requests) = QueryCache::new();
		let sender = Arc::new(InputSender::new(objects_arc, capture_requests));

		let beam_query = PION.register_object(BeamQueryCache(query_cache));
		let beam_handler_proxy = BeamQueryHandler::from_handler(&beam_query);

		let keyboard_cache = PION.register_object(KeyboardQueryCache::default());
		let keyboard_handler_proxy = PointsQueryHandler::from_handler(&keyboard_cache);

		let query_guard: Arc<OnceLock<SpatialQueryGuard>> = Arc::new(OnceLock::new());
		let points_handle: Arc<OnceLock<PointsQueryHandleProxy>> = Arc::new(OnceLock::new());
		let base_spatial_ref = SpatialRefProxy::from_handler(spatial.get_ref());
		let keyboard_spatial_ref = SpatialRefProxy::from_handler(spatial.get_ref());
		tokio::spawn({
			let query_guard = query_guard.clone();
			let points_handle = points_handle.clone();
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
					Ok(Ok(guard)) => {
						query_guard.set(guard).ok();
					}
					Ok(Err(e)) => {
						error!("failed to create mouse pointer beam query: {e}");
					}
					Err(e) => {
						error!("failed to create mouse pointer beam query: {e}");
					}
				}
				// Starts with no points — no keyboard focus until the pointer hits
				// something; update() moves the point to the beam hit each frame.
				match sqi_proxy
					.points_query(PointsQuery {
						handler: keyboard_handler_proxy,
						interfaces: vec![InterfaceDependency {
							id: KEYBOARD_PROTOCOL.protocol_name.to_string(),
							optional: false,
						}],
						reference_spatial: keyboard_spatial_ref,
						points: vec![],
					})
					.await
				{
					Ok(Ok(handle)) => {
						points_handle.set(handle).ok();
					}
					Ok(Err(e)) => {
						error!("failed to create mouse pointer keyboard query: {e}");
					}
					Err(e) => {
						error!("failed to create mouse pointer keyboard query: {e}");
					}
				}
			}
		});

		let method = PION.register_object(MouseMethod {
			spatial_arc,
			event: RwLock::new(MouseEvent::default()),
			sender,
			_beam_query: beam_query,
			_query_guard: query_guard,
		});

		let xkb_context = XkbContext::new(ContextFlags::empty())
			.map_err(|e| eyre!("failed to create xkb context: {e:?}"))?;
		let xkb_keymap = XkbKeymap::new_from_names(xkb_context, None, CompileFlags::empty())
			.map_err(|e| eyre!("failed to compile default keymap: {e:?}"))?;
		let keymap_string = xkb_keymap
			.get_as_string(KeymapFormat::TextV1)
			.map_err(|e| eyre!("failed to serialize default keymap: {e:?}"))?;
		let keyboard = KeyboardFocus {
			cache: keyboard_cache,
			points_handle,
			xkb_state: XkbState::new(xkb_keymap),
			keymap_string,
			keymap_proxy: OnceLock::new(),
		};

		Ok(MousePointer {
			spatial,
			method,
			keyboard,
		})
	}

	#[instrument(name = "update pointer", level = "debug", skip_all)]
	pub fn update(
		&mut self,
		ray: Ray3d,
		mouse_buttons: &ButtonInput<MouseButton>,
		mut scroll: EventReader<MouseWheel>,
		key_events: EventReader<KeyboardInput>,
	) {
		let mut discrete = Vec2::ZERO;
		let mut continuous = Vec2::ZERO;
		for e in scroll.read() {
			match e.unit {
				bevy::input::mouse::MouseScrollUnit::Line => {
					discrete.x += e.x;
					discrete.y += e.y;
				}
				bevy::input::mouse::MouseScrollUnit::Pixel => {
					continuous.x += e.x;
					continuous.y += e.y;
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

		self.update_keyboard_focus();
		self.send_key_events(key_events);
	}

	/// Move the keyboard focus point to the pointer's current beam hit — keyboard
	/// handlers within [`KEYBOARD_FOCUS_MARGIN`] of whatever the pointer is aimed at
	/// become focus candidates. No hit means no points and thus no focus.
	fn update_keyboard_focus(&self) {
		let Some(handle) = self.keyboard.points_handle.get() else {
			return;
		};
		let hit = self
			.method
			.sender
			.cache
			.blocking_read()
			.values()
			.filter(|e| e.spatial.is_some() && !e.left_query)
			.map(|e| e.value.deepest_point_distance)
			.min_by(f32::total_cmp);
		let points = hit
			.map(|distance| {
				vec![Point {
					point: Vec3F {
						x: 0.0,
						y: 0.0,
						z: -distance,
					},
					margin: KEYBOARD_FOCUS_MARGIN,
				}]
			})
			.unwrap_or_default();
		_ = handle.update_points(points);
	}

	fn send_key_events(&mut self, mut key_events: EventReader<KeyboardInput>) {
		for event in key_events.read() {
			if event.repeat {
				continue;
			}
			let Some(keycode) = map_key(event.key_code) else {
				warn!("unable to map key code: {:?}", event.key_code);
				continue;
			};
			let pressed = event.state.is_pressed();
			// Track modifiers even without a focused handler so the state is
			// correct once one gains focus.
			let direction = if pressed {
				KeyDirection::Down
			} else {
				KeyDirection::Up
			};
			self.keyboard.xkb_state.update_key(keycode + 8, direction);

			let Some(handler) = self.keyboard.cache.closest() else {
				continue;
			};
			let Some(keymap) = self.keyboard.keymap_proxy() else {
				continue;
			};
			let modifiers = ModifierState {
				depressed: self
					.keyboard
					.xkb_state
					.serialize_mods(StateComponent::MODS_DEPRESSED),
				latched: self
					.keyboard
					.xkb_state
					.serialize_mods(StateComponent::MODS_LATCHED),
				locked: self
					.keyboard
					.xkb_state
					.serialize_mods(StateComponent::MODS_LOCKED),
			};
			_ = handler
				.key(
					KeyEvent {
						keycode,
						pressed,
						modifiers,
						keymap,
					},
					Timestamp::now(),
				)
				.inspect_err(|err| error!("failed to send key to keyboard handler: {err}"));
		}
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
		"scroll_continuous".to_string(),
		DatamapData::Vec2 {
			value: [event.scroll_continuous.x, event.scroll_continuous.y].into(),
		},
	);
	map.insert(
		"scroll_discrete".to_string(),
		DatamapData::Vec2 {
			value: [event.scroll_discrete.x, event.scroll_discrete.y].into(),
		},
	);
	map
}

/// Map a bevy key code to a linux input event code (xkb keycode minus 8).
fn map_key(key: KeyCode) -> Option<u32> {
	use KeyCode as Key;
	match key {
		Key::Unidentified(NativeKeyCode::Xkb(code)) => Some(code),
		Key::Backspace => Some(input_event_codes::KEY_BACKSPACE!()),
		Key::Tab => Some(input_event_codes::KEY_TAB!()),
		Key::Enter => Some(input_event_codes::KEY_ENTER!()),
		Key::ShiftLeft => Some(input_event_codes::KEY_LEFTSHIFT!()),
		Key::ShiftRight => Some(input_event_codes::KEY_RIGHTSHIFT!()),
		Key::ControlLeft => Some(input_event_codes::KEY_LEFTCTRL!()),
		Key::ControlRight => Some(input_event_codes::KEY_RIGHTCTRL!()),
		Key::AltLeft => Some(input_event_codes::KEY_LEFTALT!()),
		Key::AltRight => Some(input_event_codes::KEY_RIGHTALT!()),
		Key::CapsLock => Some(input_event_codes::KEY_CAPSLOCK!()),
		Key::Escape => Some(input_event_codes::KEY_ESC!()),
		Key::Space => Some(input_event_codes::KEY_SPACE!()),
		Key::End => Some(input_event_codes::KEY_END!()),
		Key::Home => Some(input_event_codes::KEY_HOME!()),
		Key::ArrowLeft => Some(input_event_codes::KEY_LEFT!()),
		Key::ArrowRight => Some(input_event_codes::KEY_RIGHT!()),
		Key::ArrowUp => Some(input_event_codes::KEY_UP!()),
		Key::ArrowDown => Some(input_event_codes::KEY_DOWN!()),
		Key::PageUp => Some(input_event_codes::KEY_PAGEUP!()),
		Key::PageDown => Some(input_event_codes::KEY_PAGEDOWN!()),
		Key::PrintScreen => Some(input_event_codes::KEY_PRINT!()),
		Key::Insert => Some(input_event_codes::KEY_INSERT!()),
		Key::Delete => Some(input_event_codes::KEY_DELETE!()),
		Key::Digit0 => Some(input_event_codes::KEY_0!()),
		Key::Digit1 => Some(input_event_codes::KEY_1!()),
		Key::Digit2 => Some(input_event_codes::KEY_2!()),
		Key::Digit3 => Some(input_event_codes::KEY_3!()),
		Key::Digit4 => Some(input_event_codes::KEY_4!()),
		Key::Digit5 => Some(input_event_codes::KEY_5!()),
		Key::Digit6 => Some(input_event_codes::KEY_6!()),
		Key::Digit7 => Some(input_event_codes::KEY_7!()),
		Key::Digit8 => Some(input_event_codes::KEY_8!()),
		Key::Digit9 => Some(input_event_codes::KEY_9!()),
		Key::KeyA => Some(input_event_codes::KEY_A!()),
		Key::KeyB => Some(input_event_codes::KEY_B!()),
		Key::KeyC => Some(input_event_codes::KEY_C!()),
		Key::KeyD => Some(input_event_codes::KEY_D!()),
		Key::KeyE => Some(input_event_codes::KEY_E!()),
		Key::KeyF => Some(input_event_codes::KEY_F!()),
		Key::KeyG => Some(input_event_codes::KEY_G!()),
		Key::KeyH => Some(input_event_codes::KEY_H!()),
		Key::KeyI => Some(input_event_codes::KEY_I!()),
		Key::KeyJ => Some(input_event_codes::KEY_J!()),
		Key::KeyK => Some(input_event_codes::KEY_K!()),
		Key::KeyL => Some(input_event_codes::KEY_L!()),
		Key::KeyM => Some(input_event_codes::KEY_M!()),
		Key::KeyN => Some(input_event_codes::KEY_N!()),
		Key::KeyO => Some(input_event_codes::KEY_O!()),
		Key::KeyP => Some(input_event_codes::KEY_P!()),
		Key::KeyQ => Some(input_event_codes::KEY_Q!()),
		Key::KeyR => Some(input_event_codes::KEY_R!()),
		Key::KeyS => Some(input_event_codes::KEY_S!()),
		Key::KeyT => Some(input_event_codes::KEY_T!()),
		Key::KeyU => Some(input_event_codes::KEY_U!()),
		Key::KeyV => Some(input_event_codes::KEY_V!()),
		Key::KeyW => Some(input_event_codes::KEY_W!()),
		Key::KeyX => Some(input_event_codes::KEY_X!()),
		Key::KeyY => Some(input_event_codes::KEY_Y!()),
		Key::KeyZ => Some(input_event_codes::KEY_Z!()),
		Key::Numpad0 => Some(input_event_codes::KEY_NUMERIC_0!()),
		Key::Numpad1 => Some(input_event_codes::KEY_NUMERIC_1!()),
		Key::Numpad2 => Some(input_event_codes::KEY_NUMERIC_2!()),
		Key::Numpad3 => Some(input_event_codes::KEY_NUMERIC_3!()),
		Key::Numpad4 => Some(input_event_codes::KEY_NUMERIC_4!()),
		Key::Numpad5 => Some(input_event_codes::KEY_NUMERIC_5!()),
		Key::Numpad6 => Some(input_event_codes::KEY_NUMERIC_6!()),
		Key::Numpad7 => Some(input_event_codes::KEY_NUMERIC_7!()),
		Key::Numpad8 => Some(input_event_codes::KEY_NUMERIC_8!()),
		Key::Numpad9 => Some(input_event_codes::KEY_NUMERIC_9!()),
		Key::F1 => Some(input_event_codes::KEY_F1!()),
		Key::F2 => Some(input_event_codes::KEY_F2!()),
		Key::F3 => Some(input_event_codes::KEY_F3!()),
		Key::F4 => Some(input_event_codes::KEY_F4!()),
		Key::F5 => Some(input_event_codes::KEY_F5!()),
		Key::F6 => Some(input_event_codes::KEY_F6!()),
		Key::F7 => Some(input_event_codes::KEY_F7!()),
		Key::F8 => Some(input_event_codes::KEY_F8!()),
		Key::F9 => Some(input_event_codes::KEY_F9!()),
		Key::F10 => Some(input_event_codes::KEY_F10!()),
		Key::F11 => Some(input_event_codes::KEY_F11!()),
		Key::F12 => Some(input_event_codes::KEY_F12!()),
		Key::F13 => Some(input_event_codes::KEY_F13!()),
		Key::F14 => Some(input_event_codes::KEY_F14!()),
		Key::F15 => Some(input_event_codes::KEY_F15!()),
		Key::F16 => Some(input_event_codes::KEY_F16!()),
		Key::F17 => Some(input_event_codes::KEY_F17!()),
		Key::F18 => Some(input_event_codes::KEY_F18!()),
		Key::F19 => Some(input_event_codes::KEY_F19!()),
		Key::F20 => Some(input_event_codes::KEY_F20!()),
		Key::F21 => Some(input_event_codes::KEY_F21!()),
		Key::F22 => Some(input_event_codes::KEY_F22!()),
		Key::F23 => Some(input_event_codes::KEY_F23!()),
		Key::F24 => Some(input_event_codes::KEY_F24!()),
		Key::Comma => Some(input_event_codes::KEY_COMMA!()),
		Key::Period => Some(input_event_codes::KEY_DOT!()),
		Key::Slash => Some(input_event_codes::KEY_SLASH!()),
		Key::Backslash => Some(input_event_codes::KEY_BACKSLASH!()),
		Key::Semicolon => Some(input_event_codes::KEY_SEMICOLON!()),
		Key::Quote => Some(input_event_codes::KEY_APOSTROPHE!()),
		Key::BracketLeft => Some(input_event_codes::KEY_LEFTBRACE!()),
		Key::BracketRight => Some(input_event_codes::KEY_RIGHTBRACE!()),
		Key::Minus => Some(input_event_codes::KEY_MINUS!()),
		Key::Equal => Some(input_event_codes::KEY_EQUAL!()),
		Key::Backquote => Some(input_event_codes::KEY_GRAVE!()),
		Key::SuperLeft => Some(input_event_codes::KEY_LEFTMETA!()),
		Key::SuperRight => Some(input_event_codes::KEY_RIGHTMETA!()),
		Key::NumpadMultiply => Some(input_event_codes::KEY_NUMERIC_STAR!()),
		Key::NumpadAdd => Some(input_event_codes::KEY_KPPLUS!()),
		Key::NumpadSubtract => Some(input_event_codes::KEY_MINUS!()),
		Key::NumpadDecimal => Some(input_event_codes::KEY_DOT!()),
		Key::NumpadDivide => Some(input_event_codes::KEY_SLASH!()),
		Key::ContextMenu => Some(input_event_codes::KEY_CONTEXT_MENU!()),
		Key::Help => Some(input_event_codes::KEY_HELP!()),
		Key::NumLock => Some(input_event_codes::KEY_NUMLOCK!()),
		Key::NumpadBackspace => Some(input_event_codes::KEY_BACKSPACE!()),
		Key::NumpadClear => Some(input_event_codes::KEY_CLEAR!()),
		Key::NumpadClearEntry => Some(input_event_codes::KEY_CLEAR!()),
		Key::NumpadComma => Some(input_event_codes::KEY_COMMA!()),
		Key::NumpadEnter => Some(input_event_codes::KEY_ENTER!()),
		Key::NumpadEqual => Some(input_event_codes::KEY_EQUAL!()),
		Key::NumpadHash => Some(input_event_codes::KEY_NUMERIC_POUND!()),
		Key::NumpadStar => Some(input_event_codes::KEY_KPASTERISK!()),
		Key::Fn => Some(input_event_codes::KEY_FN!()),
		Key::ScrollLock => Some(input_event_codes::KEY_SCROLLLOCK!()),
		Key::Pause => Some(input_event_codes::KEY_PAUSE!()),
		Key::Power => Some(input_event_codes::KEY_POWER!()),
		Key::Sleep => Some(input_event_codes::KEY_SLEEP!()),
		Key::Suspend => Some(input_event_codes::KEY_SUSPEND!()),
		Key::Again => Some(input_event_codes::KEY_AGAIN!()),
		Key::Copy => Some(input_event_codes::KEY_COPY!()),
		Key::Cut => Some(input_event_codes::KEY_CUT!()),
		Key::Find => Some(input_event_codes::KEY_FIND!()),
		Key::Open => Some(input_event_codes::KEY_OPEN!()),
		Key::Paste => Some(input_event_codes::KEY_PASTE!()),
		Key::Props => Some(input_event_codes::KEY_PROPS!()),
		Key::Select => Some(input_event_codes::KEY_SELECT!()),
		Key::Undo => Some(input_event_codes::KEY_UNDO!()),
		Key::Hiragana => Some(input_event_codes::KEY_HIRAGANA!()),
		Key::Katakana => Some(input_event_codes::KEY_KATAKANA!()),
		_ => None,
	}
}

use super::{
	CachedHandler, DatamapBuilder, FrameDriver, InputMethod as InputMethodNode, InputMethodHelper,
	order_by_distance, spawn_frame_driver,
};
use crate::{
	bevy_int::flatscreen_cam::FlatscreenCam,
	keymap_store::KEYMAP_STORE,
	nodes::spatial::{SpatialObject, SpatialRef},
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
use gluon_ipc::{Context, Handler, Interface, RefExt};
use mint::Vector2;
use stardust_xr_molecules_protocols::keyboard_handler::{
	KeyEvent, KeyboardHandler as KeyboardHandlerProxy, ModifierState,
};
use stardust_xr_protocol::{
	field::{FieldRef as FieldRefProxy, FieldSample, RayMarchResult},
	keymap::Keymap as KeymapProxy,
	query::{InterfaceDependency, QueriedInterface, QueryableId},
	spatial::{Spatial as SpatialProxy, SpatialRef as SpatialRefProxy},
	spatial_query::{
		BeamQueryHandle, Point, PointsQuery, PointsQueryHandle as PointsQueryHandleProxy,
		PointsQueryHandler, PointsQueryHandlerHandler,
		SpatialQueryInterface as SpatialQueryInterfaceProxy,
	},
	suis::{DatamapData, InputDataType, InputHandler, Pointer},
	types::{Posef, Timestamp, Vec3F},
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
		app.add_systems(Startup, (setup, setup_capture_indicator));
		app.add_systems(
			Update,
			(
				stop_capture_hotkey,
				update_pointer,
				update_capture_indicator,
			)
				.chain(),
		);
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
	pointer.update(ray, &mouse_buttons, &keyboard_buttons, scroll, key_events);
}

/// Ctrl+Escape force-releases the active pointer capture. Runs before
/// `update_pointer` so the release is drained by this frame's `send`.
fn stop_capture_hotkey(
	keyboard_buttons: Res<ButtonInput<KeyCode>>,
	pointer: Option<Res<MousePointer>>,
) {
	let ctrl = keyboard_buttons.pressed(KeyCode::ControlLeft)
		|| keyboard_buttons.pressed(KeyCode::ControlRight);
	if !(ctrl && keyboard_buttons.just_pressed(KeyCode::Escape)) {
		return;
	}
	if let Some(pointer) = pointer {
		pointer.method.stop_active_capture();
	}
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

// ── Capture indicator ─────────────────────────────────────────────────────────

/// Bottom-right overlay showing which client currently captures the pointer.
#[derive(Component)]
struct CaptureIndicator;

fn setup_capture_indicator(mut cmds: Commands) {
	cmds.spawn((
		CaptureIndicator,
		Name::new("Capture Indicator"),
		Text::new(""),
		TextFont {
			font_size: 14.0,
			..Default::default()
		},
		TextColor(Color::WHITE),
		Node {
			position_type: PositionType::Absolute,
			bottom: Val::Px(8.0),
			right: Val::Px(8.0),
			..Default::default()
		},
	));
}

fn update_capture_indicator(
	pointer: Option<Res<MousePointer>>,
	mut text: Single<&mut Text, With<CaptureIndicator>>,
) {
	let label = pointer
		.and_then(|p| p.captured_by())
		.map(|(name, pid)| format!("captured by {name}, {pid}"))
		.unwrap_or_default();
	if text.0 != label {
		text.0 = label;
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
	handlers: Mutex<HashMap<QueryableId, (KeyboardHandlerProxy, FieldSample)>>,
}

impl KeyboardQueryCache {
	fn closest(&self) -> Option<KeyboardHandlerProxy> {
		self.handlers
			.lock()
			.unwrap()
			.values()
			.min_by(|(_, s1), (_, s2)| s1.distance.total_cmp(&s2.distance))
			.map(|(handler, _)| handler.clone())
	}
}

impl PointsQueryHandlerHandler for KeyboardQueryCache {
	async fn entered(
		&self,
		_ctx: gluon_ipc::Context,
		obj: QueryableId,
		_field: FieldRefProxy,
		_spatial: SpatialRefProxy,
		interfaces: Vec<QueriedInterface>,
		sample: FieldSample,
	) {
		let Some(interface) = interfaces.first() else {
			return;
		};
		if interface.interface_id != KeyboardHandlerProxy::ID {
			return;
		}
		let handler = KeyboardHandlerProxy::from_ref(interface.interface.clone());
		self.handlers.lock().unwrap().insert(obj, (handler, sample));
	}

	async fn interfaces_changed(
		&self,
		_ctx: gluon_ipc::Context,
		_obj: QueryableId,
		_interfaces: Vec<QueriedInterface>,
	) {
	}

	async fn moved(&self, _ctx: gluon_ipc::Context, obj: QueryableId, sample: FieldSample) {
		if let Some(entry) = self.handlers.lock().unwrap().get_mut(&obj) {
			entry.1 = sample;
		}
	}

	async fn left(&self, _ctx: gluon_ipc::Context, obj: QueryableId) {
		self.handlers.lock().unwrap().remove(&obj);
	}
}

/// Everything needed to turn bevy key events into `KeyboardHandler.key` calls:
/// the query cache above, the query's handle (to move the focus point to the
/// pointer's hit each frame), and xkb state for modifiers + the keymap token.
struct KeyboardFocus {
	cache: gluon_ipc::Node<KeyboardQueryCache>,
	points_handle: Arc<OnceLock<PointsQueryHandleProxy>>,
	xkb_state: XkbState,
	/// workaround for buggy modifier state on kde plasma (potentially others) with winit
	super_mod_mask: u32,
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
			.register(&bytes)
			.inspect_err(|err| error!("failed to register mouse pointer keymap: {err:?}"))
			.ok()?;
		_ = self.keymap_proxy.set(proxy.clone());
		Some(proxy)
	}
}

// ── MouseMethod ───────────────────────────────────────────────────────────────

#[derive(Debug)]
struct MouseMethod {
	event: RwLock<MouseEvent>,
	/// Program name + PID of each client that requested a capture, keyed by its
	/// handler; looked up when that handler's capture becomes active.
	capture_pids: Mutex<HashMap<InputHandler, (String, i32)>>,
}

impl InputMethodHelper for MouseMethod {
	type QueryValue = RayMarchResult;

	async fn order_handlers_and_captures(
		&self,
		handlers: &HashMap<QueryableId, CachedHandler<RayMarchResult>>,
		capture_requests: &HashSet<InputHandler>,
		active_capture: Option<&InputHandler>,
	) -> (Vec<InputHandler>, Option<InputHandler>) {
		order_by_distance(handlers, capture_requests, active_capture, |e| {
			if e.value.min_distance > 0.0 {
				None
			} else {
				const DEPTH_WEIGHT: f32 = 5.0;
				Some(
					e.value
						.deepest_point_distance
						.hypot(DEPTH_WEIGHT * e.value.min_distance.abs()),
				)
			}
		})
	}

	async fn input_data(&self, _time: Timestamp) -> Option<InputDataType> {
		Some(InputDataType::Pointer {
			data: Pointer {
				pose: Posef::default(),
				deepest_point: 0.0,
			},
		})
	}

	async fn datamap(&self) -> HashMap<String, DatamapData> {
		build_datamap(&*self.event.read().await)
	}

	async fn capture_granted(&self, ctx: &Context, handler: &InputHandler) {
		let pid = ctx.sender_pid().unwrap_or(-1);
		let name = std::fs::read_to_string(format!("/proc/{pid}/comm"))
			.map(|s| s.trim().to_string())
			.unwrap_or_else(|_| "unknown".to_string());
		self.capture_pids
			.lock()
			.unwrap()
			.insert(handler.clone(), (name, pid));
	}
}

// ── MousePointer ──────────────────────────────────────────────────────────────

#[derive(Resource)]
pub struct MousePointer {
	spatial: gluon_ipc::LocalRef<SpatialProxy, SpatialObject>,
	method: gluon_ipc::Node<InputMethodNode<MouseMethod>>,
	driver: FrameDriver,
	_beam_handle: Arc<OnceLock<BeamQueryHandle>>,
	keyboard: KeyboardFocus,
	/// An Escape press was swallowed as part of the Ctrl+Escape capture-stop
	/// hotkey; swallow its release too (even if Ctrl is let go first).
	swallow_escape_release: bool,
}

impl MousePointer {
	pub fn new() -> Result<Self> {
		let spatial = SpatialObject::new(None, Mat4::IDENTITY);
		let spatial_ref = spatial.get_ref().proxy().clone();

		let (method, _, beam_handle) = InputMethodNode::new_beam(
			MouseMethod {
				event: RwLock::new(MouseEvent::default()),
				capture_pids: Mutex::new(HashMap::new()),
			},
			(**spatial).clone(),
			spatial_ref.clone(),
			Vec3F::from([0.0, 0.0, 0.0]),
			Vec3F::from([0.0, 0.0, -1.0]),
			f32::MAX,
			0.0,
		)?;
		let driver = spawn_frame_driver(&method);

		let (keyboard_cache, keyboard_handler_proxy) =
			PointsQueryHandler::new_node(KeyboardQueryCache::default())?;
		let points_handle: Arc<OnceLock<PointsQueryHandleProxy>> = Arc::new(OnceLock::new());
		tokio::spawn({
			let points_handle = points_handle.clone();
			async move {
				let sqi = SpatialQueryInterfaceProxy::new_service(SpatialQueryInterface::new(
					&Arc::default(),
				))
				// TODO: remove the unwrap
				.unwrap()
				.into_proxy();
				// Starts with no points — no keyboard focus until the pointer hits
				// something; update() moves the point to the beam hit each frame.
				match sqi
					.points_query(PointsQuery {
						handler: keyboard_handler_proxy.into_proxy(),
						interfaces: vec![InterfaceDependency {
							id: KeyboardHandlerProxy::ID.into(),
							optional: false,
						}],
						reference_spatial: spatial_ref,
						points: vec![],
					})
					.await
				{
					Ok(Ok(handle)) => {
						points_handle.set(handle).ok();
					}
					Ok(Err(e)) => error!("failed to create mouse pointer keyboard query: {e}"),
					Err(e) => error!("failed to create mouse pointer keyboard query: {e}"),
				}
			}
		});

		let xkb_context = XkbContext::new(ContextFlags::empty())
			.map_err(|e| eyre!("failed to create xkb context: {e:?}"))?;
		let xkb_keymap = XkbKeymap::new_from_names(xkb_context, None, CompileFlags::empty())
			.map_err(|e| eyre!("failed to compile default keymap: {e:?}"))?;
		let keymap_string = xkb_keymap
			.get_as_string(KeymapFormat::TextV1)
			.map_err(|e| eyre!("failed to serialize default keymap: {e:?}"))?;

		let super_mod_mask = !xkb_keymap
			.mod_get_index("Mod4")
			.map(|v| 1u32 << v)
			.unwrap_or(0);
		let keyboard = KeyboardFocus {
			cache: keyboard_cache,
			points_handle,
			xkb_state: XkbState::new(xkb_keymap),
			keymap_string,
			keymap_proxy: OnceLock::new(),
			super_mod_mask,
		};

		Ok(MousePointer {
			spatial,
			method,
			driver,
			_beam_handle: beam_handle,
			keyboard,
			swallow_escape_release: false,
		})
	}

	#[instrument(name = "update pointer", level = "debug", skip_all)]
	pub fn update(
		&mut self,
		ray: Ray3d,
		mouse_buttons: &ButtonInput<MouseButton>,
		keyboard_buttons: &ButtonInput<KeyCode>,
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

		self.driver.frame(Timestamp::now());

		self.update_keyboard_focus();
		let ctrl_pressed = keyboard_buttons.pressed(KeyCode::ControlLeft)
			|| keyboard_buttons.pressed(KeyCode::ControlRight);
		self.send_key_events(key_events, ctrl_pressed);

		// Drop pid entries for handlers whose capture request is gone (released or client
		// died); the frame driver's send is what sweeps capture_requests.
		{
			let requests = self.method.cache().capture_requests_blocking();
			self.method
				.capture_pids
				.lock()
				.unwrap()
				.retain(|handler, _| requests.contains(handler));
		}
	}

	/// Program name and PID of the client whose handler currently captures the
	/// pointer, if any.
	pub fn captured_by(&self) -> Option<(String, i32)> {
		let capture = self.method.active_capture_blocking()?;
		self.method
			.capture_pids
			.lock()
			.unwrap()
			.get(&capture)
			.cloned()
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
			.cache()
			.handlers_blocking()
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
		_ = handle.update(points);
	}

	fn send_key_events(&mut self, mut key_events: EventReader<KeyboardInput>, ctrl_pressed: bool) {
		for event in key_events.read() {
			if event.repeat {
				continue;
			}
			// Ctrl+Escape is the capture-stop hotkey (see stop_capture_hotkey);
			// swallow the press and its matching release so focused keyboard
			// handlers never see it.
			if event.key_code == KeyCode::Escape {
				if event.state.is_pressed() && ctrl_pressed {
					self.swallow_escape_release = true;
					continue;
				}
				if !event.state.is_pressed() && self.swallow_escape_release {
					self.swallow_escape_release = false;
					continue;
				}
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
			let mod_mask = self.keyboard.super_mod_mask;
			let modifiers = ModifierState {
				depressed: self
					.keyboard
					.xkb_state
					.serialize_mods(StateComponent::MODS_DEPRESSED)
					& mod_mask,
				latched: self
					.keyboard
					.xkb_state
					.serialize_mods(StateComponent::MODS_LATCHED)
					& mod_mask,
				locked: self
					.keyboard
					.xkb_state
					.serialize_mods(StateComponent::MODS_LOCKED)
					& mod_mask,
				layout_group: self
					.keyboard
					.xkb_state
					.serialize_layout(StateComponent::LAYOUT_EFFECTIVE) as u32,
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
	DatamapBuilder::default()
		.f32("select", event.select)
		.f32("middle", event.middle)
		.f32("context", event.context)
		.f32("grab", event.grab)
		.vec2("scroll_continuous", event.scroll_continuous)
		.vec2("scroll_discrete", event.scroll_discrete)
		.build()
}

/// Map a bevy key code to a linux input event code (xkb keycode minus 8).
fn map_key(key: KeyCode) -> Option<u32> {
	use KeyCode as Key;
	match key {
		Key::Unidentified(NativeKeyCode::Xkb(code)) => Some(code - 8),
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

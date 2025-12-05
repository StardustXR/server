use super::{CaptureManager, DistanceCalculator, get_sorted_handlers};
use crate::{
	DbusConnection, ObjectRegistryRes,
	core::{client::INTERNAL_CLIENT, task},
	nodes::{
		Node, OwnedNode,
		fields::{EXPORTED_FIELDS, Field, FieldTrait, Ray},
		input::{InputDataType, InputMethod, Pointer},
		items::panel::KEYMAPS,
		spatial::Spatial,
	},
	objects::FieldRef,
};
use bevy::{
	input::{
		ButtonState,
		keyboard::{KeyboardInput, NativeKey, NativeKeyCode},
		mouse::{MouseMotion, MouseWheel},
	},
	prelude::*,
	window::PrimaryWindow,
};
use color_eyre::eyre::Result;
use dashmap::DashMap;
use glam::{Mat4, Vec3, vec3};
use mint::Vector2;
use rustc_hash::{FxHashMap, FxHasher};
use serde::{Deserialize, Serialize};
use slotmap::{DefaultKey, Key as SlotKey};
use stardust_xr_gluon::{
	ObjectInfo,
	interfaces::FieldRefProxy,
	list_query::{ListEvent, ObjectListQuery},
	object_registry::ObjectRegistry,
	query::{ObjectQuery, QueryContext, QueryEvent},
};
use stardust_xr_wire::values::Datamap;
use std::sync::{Arc, Weak};
use tokio::sync::{Notify, mpsc, watch};
use tokio::task::{AbortHandle, JoinSet};
use tokio::time::{Duration, timeout};
use xkbcommon_rs::{Context, Keymap, KeymapFormat, xkb_keymap::CompileFlags};
use zbus::{Connection, names::OwnedInterfaceName};

#[derive(Clone)]
struct HandlerInfo {
	handler: ObjectInfo,
	field_ref: Arc<Field>,
	keyboard_proxy: KeyboardHandlerProxy<'static>,
}

#[derive(Debug, Clone)]
struct InputEvent {
	key: u32,
	pressed: bool,
}

pub struct FlatscreenInputPlugin;
impl Plugin for FlatscreenInputPlugin {
	fn build(&self, app: &mut App) {
		app.add_systems(Startup, setup);
		// yes the input method will be delayed by one frame, its only for debugging anyways
		app.add_systems(Update, update_pointer);
	}
}

#[derive(Component)]
#[require(Camera3d)]
pub struct FlatscreenCam;

fn setup(mut cmds: Commands, object_registry: Res<ObjectRegistryRes>) {
	let Ok(pointer) = MousePointer::new(object_registry.0.clone())
		.inspect_err(|err| error!("unable to create mouse pointer: {err}"))
	else {
		return;
	};
	cmds.spawn((FlatscreenCam, Name::new("Flatscreen Camera")));
	cmds.insert_resource(pointer);
}

fn update_pointer(
	window: Single<(&Window), With<PrimaryWindow>>,
	mut cam: Single<(&Camera, &GlobalTransform, &mut Transform), With<FlatscreenCam>>,
	mut pointer: ResMut<MousePointer>,
	connection: Res<DbusConnection>,
	object_registry: Res<ObjectRegistryRes>,
	mouse_buttons: Res<ButtonInput<MouseButton>>,
	keyboard_buttons: Res<ButtonInput<KeyCode>>,
	mut scroll: EventReader<MouseWheel>,
	mut motion: EventReader<MouseMotion>,
	mut keyboard_input_events: EventReader<KeyboardInput>,
	time: Res<Time>,
) {
	let (cam, cam_transform, mut cam_local_transform) = cam.into_inner();
	if keyboard_buttons.pressed(KeyCode::ShiftLeft) && mouse_buttons.pressed(MouseButton::Right) {
		let (mut yaw, mut pitch, _) = cam_local_transform.rotation.to_euler(EulerRot::YXZ);

		for e in motion.read() {
			let scale = -0.003;
			pitch += e.delta.y * scale;
			yaw += e.delta.x * scale;
		}

		cam_local_transform.rotation = Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch);

		let mut move_vec = Vec3::ZERO;
		move_vec.x += keyboard_buttons.pressed(KeyCode::KeyD) as u32 as f32;
		move_vec.x -= keyboard_buttons.pressed(KeyCode::KeyA) as u32 as f32;
		move_vec.z += keyboard_buttons.pressed(KeyCode::KeyS) as u32 as f32;
		move_vec.z -= keyboard_buttons.pressed(KeyCode::KeyW) as u32 as f32;
		move_vec.y += keyboard_buttons.pressed(KeyCode::KeyE) as u32 as f32;
		move_vec.y -= keyboard_buttons.pressed(KeyCode::KeyQ) as u32 as f32;

		let move_vec = cam_local_transform.rotation * move_vec.normalize_or_zero();
		cam_local_transform.translation += move_vec * time.delta_secs() * 3.0;

		return;
	}
	let Some(ray) = window
		.cursor_position()
		.and_then(|pos| get_viewport_pos(pos, cam))
		.and_then(|pos| cam.viewport_to_world(cam_transform, pos).ok())
	else {
		return;
	};
	pointer.update(
		&connection,
		&object_registry,
		ray,
		&mouse_buttons,
		&keyboard_buttons,
		scroll,
		keyboard_input_events,
	);
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

#[derive(Debug, Deserialize, Serialize)]
struct MouseEvent {
	select: f32,
	middle: f32,
	context: f32,
	grab: f32,
	scroll_continuous: Vector2<f32>,
	scroll_discrete: Vector2<f32>,
	raw_input_events: Vec<u32>,
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
			raw_input_events: vec![],
		}
	}
}

#[zbus::proxy(
	interface = "org.stardustxr.XKBv1",
	default_service = "org.stardustxr.XKBv1"
)]
trait KeyboardHandler {
	async fn keymap(&self, keymap_id: u64) -> zbus::Result<()>;
	async fn key_state(&self, key: u32, pressed: bool) -> zbus::Result<()>;
	async fn reset(&self) -> zbus::Result<()>;
}

// Make KeyboardHandlerProxy queryable
stardust_xr_gluon::impl_queryable_for_proxy!(KeyboardHandlerProxy);

// Query context for keyboard handlers
#[derive(Debug, Clone)]
struct KeyboardQueryContext;
impl QueryContext for KeyboardQueryContext {}

#[derive(Resource)]
pub struct MousePointer {
	node: OwnedNode,
	keymap: DefaultKey,
	spatial: Arc<Spatial>,
	pointer: Arc<InputMethod>,
	capture_manager: CaptureManager,
	mouse_datamap: MouseEvent,
	// Task management
	focus_task_abort_handle: AbortHandle,
	input_delivery_task_abort_handle: AbortHandle,
	// Channels
	input_event_tx: mpsc::UnboundedSender<InputEvent>,
	// Notification for focus recalculation
	focus_notify: Arc<Notify>,
}
impl MousePointer {
	pub fn new(object_registry: Arc<ObjectRegistry>) -> Result<Self> {
		let node = Node::generate(&INTERNAL_CLIENT, false).add_to_scenegraph_owned()?;
		let spatial = Spatial::add_to(&node.0, None, Mat4::IDENTITY);
		let pointer = InputMethod::add_to(
			&node.0,
			InputDataType::Pointer(Pointer::default()),
			Datamap::from_typed(MouseEvent::default())?,
		)?;

		let context = Context::new(0).unwrap();
		let keymap = KEYMAPS.lock().insert(
			Keymap::new_from_names(context, None, CompileFlags::NO_FLAGS)
				.unwrap()
				.get_as_string(KeymapFormat::TextV1)
				.unwrap(),
		);

		// Create channels and notification
		let (focused_handler_tx, focused_handler_rx) = watch::channel::<Option<HandlerInfo>>(None);
		let (input_event_tx, input_event_rx) = mpsc::unbounded_channel::<InputEvent>();
		let focus_notify = Arc::new(Notify::new());
		// Spawn input delivery task
		info!("Creating input delivery task");
		let input_delivery_task_abort_handle = task::new(
			|| "Mouse pointer input delivery task",
			Self::input_delivery_task(
				object_registry.get_connection().clone(),
				focused_handler_rx,
				input_event_rx,
				keymap.data().as_ffi(),
			),
		)?
		.abort_handle();
		info!("Input delivery task created successfully");

		// Spawn focus tracking task
		info!("Creating focus tracking task");
		let focus_task_abort_handle = task::new(
			|| "Mouse pointer focus task",
			Self::focus_tracking_task(
				object_registry,
				focus_notify.clone(),
				spatial.clone(),
				pointer.clone(),
				focused_handler_tx,
			),
		)?
		.abort_handle();
		info!("Focus tracking task created successfully");

		Ok(MousePointer {
			node,
			spatial,
			pointer,
			capture_manager: CaptureManager::default(),
			mouse_datamap: Default::default(),
			keymap,
			focus_task_abort_handle,
			input_delivery_task_abort_handle,
			input_event_tx,
			focus_notify,
		})
	}
	pub fn update(
		&mut self,
		dbus_connection: &Connection,
		object_registry: &ObjectRegistry,
		ray: Ray3d,
		mouse_buttons: &ButtonInput<MouseButton>,
		keyboard_buttons: &ButtonInput<KeyCode>,
		mut scroll: EventReader<MouseWheel>,
		mut keyboard_input_events: EventReader<KeyboardInput>,
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
		{
			// Set pointer input datamap
			self.mouse_datamap = MouseEvent {
				select: mouse_buttons.pressed(MouseButton::Left) as u32 as f32,
				middle: mouse_buttons.pressed(MouseButton::Middle) as u32 as f32,
				context: mouse_buttons.pressed(MouseButton::Right) as u32 as f32,
				grab: mouse_buttons.pressed(MouseButton::Right) as u32 as f32, // Was Mouse 5
				scroll_continuous: continuous.into(),
				scroll_discrete: discrete.into(),
				raw_input_events: mouse_buttons
					.get_pressed()
					.map(|button| match button {
						MouseButton::Left => input_event_codes::BTN_LEFT!(),
						MouseButton::Right => input_event_codes::BTN_RIGHT!(),
						MouseButton::Middle => input_event_codes::BTN_MIDDLE!(),
						MouseButton::Back => input_event_codes::BTN_BACK!(),
						MouseButton::Forward => input_event_codes::BTN_FORWARD!(),
						MouseButton::Other(b) => *b as u32,
					})
					.collect(),
			};
			*self.pointer.datamap.lock() = Datamap::from_typed(&self.mouse_datamap).unwrap();
		}
		self.target_pointer_input();

		// Send keyboard input events via channel
		for event in keyboard_input_events.read() {
			if let Some(key) = map_key(event.key_code) {
				let input_event = InputEvent {
					key,
					pressed: matches!(event.state, ButtonState::Pressed),
				};
				info!(
					"Sending keyboard input event: key={}, pressed={}",
					key, input_event.pressed
				);
				if let Err(e) = self.input_event_tx.send(input_event) {
					error!("Failed to send keyboard input event: {}", e);
				}
			} else {
				warn!("Unable to map key code: {:?}", event.key_code);
			}
		}

		// Notify focus tracking task to recalculate focus
		self.focus_notify.notify_waiters();
	}
	fn target_pointer_input(&mut self) {
		let distance_calculator: DistanceCalculator = |space, data, field| {
			let result = field.ray_march(Ray {
				origin: vec3(0.0, 0.0, 0.0),
				direction: vec3(0.0, 0.0, -1.0),
				space: space.clone(),
			});
			let valid =
				result.deepest_point_distance > 0.0 && result.min_distance.is_sign_negative();
			valid.then_some(result.deepest_point_distance)
		};

		self.capture_manager.update_capture(&self.pointer);
		self.capture_manager
			.set_new_capture(&self.pointer, distance_calculator);
		self.capture_manager.apply_capture(&self.pointer);

		if self.capture_manager.capture.upgrade().is_some() {
			return;
		}

		let mut handlers = get_sorted_handlers(&self.pointer, distance_calculator);
		let first_distance = handlers
			.first()
			.map(|(_, distance)| *distance)
			.unwrap_or(f32::NEG_INFINITY);

		self.pointer.set_handler_order(
			handlers
				.iter()
				.filter(|(handler, distance)| (distance - first_distance).abs() <= 0.001)
				.map(|(handler, _)| handler),
		);
	}

	async fn focus_tracking_task(
		object_registry: Arc<ObjectRegistry>,
		focus_notify: Arc<Notify>,
		spatial: Arc<Spatial>,
		pointer: Arc<InputMethod>,
		focused_handler_tx: watch::Sender<Option<HandlerInfo>>,
	) {
		info!("Focus tracking task started");

		// Create keyboard handler query inside the task
		let mut keyboard_query = ObjectQuery::<
			(FieldRefProxy<'static>, KeyboardHandlerProxy<'static>),
			_,
		>::new(object_registry.clone(), ());
		let (keyboard_handlers, mapper) = keyboard_query.to_list_query();
		task::new(
			|| "Focus tracking mapper",
			mapper.init(async |ev| match ev {
				ListEvent::NewMatch((field_ref, keyboard_proxy)) => {
					info!("New keyboard handler found");
					let uid = timeout(Duration::from_millis(100), field_ref.uid())
						.await
						.ok()?
						.ok()?;
					let field_node = EXPORTED_FIELDS.lock().get(&uid)?.upgrade()?;
					let field = field_node.get_aspect::<Field>();
					Some((field, keyboard_proxy))
				}
				ListEvent::Modified((field_ref, keyboard_proxy)) => {
					let uid = timeout(Duration::from_millis(100), field_ref.uid())
						.await
						.ok()?
						.ok()?;
					let field_node = EXPORTED_FIELDS.lock().get(&uid)?.upgrade()?;
					let field = field_node.get_aspect::<Field>();
					Some((field, keyboard_proxy))
				}
				_ => None,
			}),
		);

		// Main focus calculation loop
		loop {
			let mut closest_handler = None;
			let mut closest_distance = f32::MAX;

			// Find closest handler
			for (handler, (field_ref, keyboard_proxy)) in &*keyboard_handlers.iter().await {
				let Ok(field_ref) = field_ref else {
					continue;
				};

				let result = field_ref.ray_march(Ray {
					origin: vec3(0.0, 0.0, 0.0),
					direction: vec3(0.0, 0.0, -1.0),
					space: spatial.clone(),
				});

				if result.deepest_point_distance > 0.0
					&& result.min_distance < 0.05
					&& result.deepest_point_distance < closest_distance
				{
					closest_distance = result.deepest_point_distance;
					closest_handler = Some(HandlerInfo {
						handler: handler.clone(),
						field_ref: field_ref.clone(),
						keyboard_proxy: keyboard_proxy.clone(),
					});
				}
			}

			// Update focused handler
			if let Some(ref handler_info) = closest_handler {
				info!(
					"Focus tracking task: Focused on handler at distance {}",
					closest_distance
				);
			} else {
				debug!("Focus tracking task: No handler in focus");
			}
			let _ = focused_handler_tx.send(closest_handler);

			// Wait for next frame signal
			focus_notify.notified().await;
		}
	}

	async fn input_delivery_task(
		dbus_connection: Connection,
		mut focused_handler_rx: watch::Receiver<Option<HandlerInfo>>,
		mut input_event_rx: mpsc::UnboundedReceiver<InputEvent>,
		keymap_id: u64,
	) {
		info!("Input delivery task started");
		loop {
			// Handle input events
			while let Some(input_event) = input_event_rx.recv().await {
				info!(
					"Input delivery task: Received input event key={}, pressed={}",
					input_event.key, input_event.pressed
				);
				// Get current focused handler
				let current_handler = focused_handler_rx.borrow().clone();
				let Some(handler_info) = current_handler else {
					continue;
				};

				// Send input to handler using cached proxy
				info!("Input delivery task: Sending to handler");
				let keyboard_handler = &handler_info.keyboard_proxy;

				// Register keymap first
				if let Err(e) = keyboard_handler.keymap(keymap_id).await {
					warn!("Input delivery task: Failed to register keymap: {}", e);
				}

				// Send key state
				if let Err(e) = keyboard_handler
					.key_state(input_event.key + 8, input_event.pressed)
					.await
				{
					error!("Input delivery task: Failed to send key state: {}", e);
				} else {
					info!(
						"Input delivery task: Successfully sent key {} (pressed={})",
						input_event.key + 8,
						input_event.pressed
					);
				}
			}
		}
	}
}

impl Drop for MousePointer {
	fn drop(&mut self) {
		// Abort the persistent tasks when MousePointer is dropped
		self.focus_task_abort_handle.abort();
		self.input_delivery_task_abort_handle.abort();
	}
}

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

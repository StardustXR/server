use super::{
	seat::{KeyboardEvent, PointerEvent, SeatData},
	state::ClientState,
};
use crate::{
	nodes::{
		drawable::model::ModelPart,
		items::panel::{Backend, Geometry, PanelItem, PanelItemInitData, SurfaceID, ToplevelInfo},
	},
	wayland::surface::CoreSurface,
};
use color_eyre::eyre::Result;
use mint::Vector2;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use smithay::{
	reexports::{
		calloop::{EventLoop, LoopSignal},
		wayland_server::{protocol::wl_surface::WlSurface, DisplayHandle, Resource},
		x11rb::protocol::xproto::Window,
	},
	utils::{Logical, Rectangle},
	wayland::compositor,
	xwayland::{
		xwm::{Reorder, ResizeEdge, XwmId},
		X11Surface, X11Wm, XWayland, XWaylandEvent, XwmHandler,
	},
};
use std::{ffi::OsStr, iter::empty, sync::Arc, time::Duration};
use tokio::sync::oneshot;
use tracing::debug;

pub static DISPLAY: OnceCell<String> = OnceCell::new();

pub struct XWaylandState {
	pub display: u32,
	event_loop_signal: LoopSignal,
}
impl XWaylandState {
	pub fn create(dh: &DisplayHandle) -> Result<Self> {
		let dh = dh.clone();

		let (tx, rx) = oneshot::channel();

		tokio::task::spawn_blocking(move || {
			let mut event_loop: EventLoop<XWaylandHandler> = EventLoop::try_new()?;
			let (xwayland, connection) = XWayland::new(&dh);
			let handle = event_loop.handle();
			event_loop
				.handle()
				.insert_source(connection, {
					let dh = dh.clone();
					move |event, _, handler| match event {
						XWaylandEvent::Ready {
							connection,
							client,
							client_fd: _,
							display: _,
						} => {
							handler.seat = client.get_data::<ClientState>().map(|s| s.seat.clone());
							handler.wm =
								X11Wm::start_wm(handle.clone(), dh.clone(), connection, client)
									.ok();
						}
						XWaylandEvent::Exited => (),
					}
				})
				.map_err(|e| e.error)?;

			let display = xwayland.start(
				event_loop.handle(),
				None,
				empty::<(&OsStr, &OsStr)>(),
				true,
				|_| (),
			)?;
			let _ = tx.send(XWaylandState {
				display,
				event_loop_signal: event_loop.get_signal(),
			});
			let mut handler = XWaylandHandler {
				wayland_display_handle: dh,
				wm: None,
				seat: None,
			};
			event_loop.run(Duration::from_millis(100), &mut handler, |_| ())
		});

		let state = rx.blocking_recv()?;
		let _ = DISPLAY.set(format!(":{}", state.display));

		Ok(state)
	}
}
impl Drop for XWaylandState {
	fn drop(&mut self) {
		self.event_loop_signal.stop();
	}
}

struct XWaylandHandler {
	wayland_display_handle: DisplayHandle,
	wm: Option<X11Wm>,
	seat: Option<Arc<SeatData>>,
}
impl XWaylandHandler {
	fn panel_item(&self, window: &X11Surface) -> Option<Arc<PanelItem<X11Backend>>> {
		compositor::with_states(&window.wl_surface()?, |s| {
			s.data_map.get::<Arc<PanelItem<X11Backend>>>().cloned()
		})
	}
}

impl XwmHandler for XWaylandHandler {
	fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
		self.wm.as_mut().unwrap()
	}

	fn new_window(&mut self, _xwm: XwmId, window: X11Surface) {
		debug!(?window, "New X window");
	}

	fn new_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
		debug!(?window, "New X override redirect window");
	}

	fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
		debug!(?window, "X map window request");
		window.set_mapped(true).unwrap();
	}
	fn map_window_notify(&mut self, _xwm: XwmId, window: X11Surface) {
		debug!(?window, "X map window notify");

		let dh = self.wayland_display_handle.clone();
		let seat = self.seat.clone().unwrap();
		CoreSurface::add_to(
			self.wayland_display_handle.clone(),
			&window.wl_surface().unwrap(),
			{
				let window = window.clone();
				move || {
					let Some(wl_surface) = window.wl_surface() else {return};
					let seat = seat.clone();
					window.user_data().insert_if_missing_threadsafe(|| {
						let (_node, panel_item) = PanelItem::create(
							Box::new(X11Backend {
								toplevel_parent: None,
								toplevel: window.clone(),
								seat,
								_pointer_grab: Mutex::new(None),
								_keyboard_grab: Mutex::new(None),
							}),
							wl_surface
								.client()
								.and_then(|c| c.get_credentials(&dh).ok())
								.map(|c| c.pid),
						);
						panel_item
					});
				}
			},
			move |_| {
				let Some(panel_item) = window.user_data().get::<Arc<PanelItem<X11Backend>>>() else {return};
				panel_item.toplevel_size_changed(
					[
						window.geometry().size.w as u32,
						window.geometry().size.h as u32,
					]
					.into(),
				);
			},
		);
	}

	fn mapped_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
		debug!(?window, "X map override redirect window");
	}

	fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
		debug!(?window, "Unmap X window");
	}

	fn destroyed_window(&mut self, _xwm: XwmId, window: X11Surface) {
		debug!(?window, "Destroy X window");
	}

	fn configure_request(
		&mut self,
		_xwm: XwmId,
		window: X11Surface,
		x: Option<i32>,
		y: Option<i32>,
		w: Option<u32>,
		h: Option<u32>,
		reorder: Option<Reorder>,
	) {
		debug!(?window, x, y, w, h, ?reorder, "Configure X window");
	}

	fn configure_notify(
		&mut self,
		_xwm: XwmId,
		window: X11Surface,
		geometry: Rectangle<i32, Logical>,
		above: Option<Window>,
	) {
		debug!(?window, ?geometry, above, "Configure X window");
	}

	fn move_request(&mut self, _xwm: XwmId, window: X11Surface, button: u32) {
		let Some(panel_item) = self.panel_item(&window) else {return};
		debug!(?window, button, "X window requests move");
		panel_item.toplevel_move_request();
	}
	fn resize_request(
		&mut self,
		_xwm: XwmId,
		window: X11Surface,
		button: u32,
		resize_edge: ResizeEdge,
	) {
		let Some(panel_item) = self.panel_item(&window) else {return};
		debug!(?window, button, ?resize_edge, "X window requests resize");
		let (up, down, left, right) = match resize_edge {
			ResizeEdge::Top => (true, false, false, false),
			ResizeEdge::Bottom => (false, true, false, false),
			ResizeEdge::Left => (false, false, true, false),
			ResizeEdge::TopLeft => (true, false, true, false),
			ResizeEdge::BottomLeft => (false, true, true, false),
			ResizeEdge::Right => (false, false, false, true),
			ResizeEdge::TopRight => (true, false, false, true),
			ResizeEdge::BottomRight => (false, true, false, true),
			// _ => (false, false, false, false),
		};
		panel_item.toplevel_resize_request(up, down, left, right)
	}

	fn fullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
		let _ = window.set_fullscreen(true);
		let Some(panel_item) = self.panel_item(&window) else {return};
		panel_item.toplevel_fullscreen_active(true);
	}
	fn unfullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
		let _ = window.set_fullscreen(false);
		let Some(panel_item) = self.panel_item(&window) else {return};
		panel_item.toplevel_fullscreen_active(true);
	}
}

pub struct X11Backend {
	pub toplevel_parent: Option<X11Surface>,
	pub toplevel: X11Surface,
	pub seat: Arc<SeatData>,
	_pointer_grab: Mutex<Option<SurfaceID>>,
	_keyboard_grab: Mutex<Option<SurfaceID>>,
}
impl X11Backend {
	fn wl_surface_from_id(&self, id: &SurfaceID) -> Option<WlSurface> {
		match id {
			SurfaceID::Cursor => None,
			SurfaceID::Toplevel => self.toplevel.wl_surface(),
			SurfaceID::Child(_) => None,
		}
	}

	// fn flush_client(&self) {
	// 	let Some(client) = self.toplevel.wl_surface().and_then(|s| s.client()) else {return};
	// 	if let Some(client_state) = client.get_data::<ClientState>() {
	// 		client_state.flush();
	// 	}
	// }
}
impl Backend for X11Backend {
	// fn start_data(&self, id: &str) -> Result<Message> {
	// 	let size = (
	// 		self.toplevel.geometry().size.w as u32,
	// 		self.toplevel.geometry().size.h as u32,
	// 	);
	// 	let toplevel_state = (
	// 		None::<String>,
	// 		self.toplevel.title(),
	// 		None::<String>,
	// 		(
	// 			self.toplevel.geometry().size.w as u32,
	// 			self.toplevel.geometry().size.h as u32,
	// 		),
	// 		self.toplevel.min_size().map(|s| (s.w as u32, s.h as u32)),
	// 		self.toplevel.max_size().map(|s| (s.w as u32, s.w as u32)),
	// 		((0_i32, 0_i32), size),
	// 		vec![0_u32; 0],
	// 	);
	// 	let info = (
	// 		None::<(Vector2<u32>, Vector2<i32>)>,
	// 		toplevel_state,
	// 		Vec::<PopupData>::new(),
	// 		None::<SurfaceID>,
	// 		None::<SurfaceID>,
	// 	);
	// 	Ok(serialize((id, info))?.into())
	// }
	// fn serialize_toplevel(&self) -> Result<Message> {
	// 	let toplevel_state = (
	// 		None::<String>,
	// 		self.toplevel.title(),
	// 		None::<String>,
	// 		(
	// 			self.toplevel.geometry().size.w,
	// 			self.toplevel.geometry().size.h,
	// 		),
	// 		self.toplevel.min_size().map(|s| (s.w, s.h)),
	// 		self.toplevel.max_size().map(|s| (s.w, s.w)),
	// 	);
	// 	let data = serialize(&toplevel_state)?;
	// 	Ok(data.into())
	// }

	// fn set_toplevel_capabilities(&self, _capabilities: Vec<u8>) {}

	// fn set_toplevel_size(
	// 	&self,
	// 	size: Option<Vector2<u32>>,
	// 	states: Vec<u32>,
	// 	_bounds: Option<Vector2<u32>>,
	// ) {
	// 	let _ = self.toplevel.configure(
	// 		size.map(|s| Rectangle::from_loc_and_size((0, 0), (s.x as i32, s.y as i32))),
	// 	);
	// 	let _ = self.toplevel.set_maximized(states.contains(&1));
	// }

	fn start_data(&self) -> Result<PanelItemInitData> {
		Ok(PanelItemInitData {
			cursor: None,
			toplevel: ToplevelInfo {
				parent: None,
				title: Some(self.toplevel.title()),
				app_id: Some(self.toplevel.instance()),
				size: [
					self.toplevel.geometry().size.w as u32,
					self.toplevel.geometry().size.h as u32,
				]
				.into(),
				min_size: self
					.toplevel
					.min_size()
					.map(|s| [s.w as u32, s.h as u32].into()),
				max_size: self
					.toplevel
					.max_size()
					.map(|s| [s.w as u32, s.h as u32].into()),
				logical_rectangle: Geometry {
					origin: [0, 0].into(),
					size: [
						self.toplevel.geometry().size.w as u32,
						self.toplevel.geometry().size.h as u32,
					]
					.into(),
				},
			},
			children: vec![],
			pointer_grab: self._pointer_grab.lock().clone(),
			keyboard_grab: self._keyboard_grab.lock().clone(),
		})
	}
	fn close_toplevel(&self) {}

	fn auto_size_toplevel(&self) {
		let _ = self.toplevel.configure(None);
	}
	fn set_toplevel_size(&self, size: Vector2<u32>) {
		let _ = self.toplevel.configure(Some(Rectangle {
			loc: self.toplevel.geometry().loc,
			size: (size.x as i32, size.y as i32).into(),
		}));
	}
	fn set_toplevel_focused_visuals(&self, focused: bool) {
		let _ = self.toplevel.set_activated(focused);
	}

	fn apply_surface_material(&self, surface: SurfaceID, model_part: &Arc<ModelPart>) {
		let Some(wl_surface) = self.wl_surface_from_id(&surface) else {return};
		let Some(core_surface) = CoreSurface::from_wl_surface(&wl_surface) else {return};

		core_surface.apply_material(model_part);
	}

	fn pointer_motion(&self, surface: &SurfaceID, position: Vector2<f32>) {
		let Some(surface) = self.wl_surface_from_id(surface) else {return};
		self.seat
			.pointer_event(&surface, PointerEvent::Motion(position));
	}
	fn pointer_button(&self, surface: &SurfaceID, button: u32, pressed: bool) {
		let Some(surface) = self.wl_surface_from_id(surface) else {return};
		self.seat.pointer_event(
			&surface,
			PointerEvent::Button {
				button,
				state: if pressed { 1 } else { 0 },
			},
		)
	}
	fn pointer_scroll(
		&self,
		surface: &SurfaceID,
		scroll_distance: Option<Vector2<f32>>,
		scroll_steps: Option<Vector2<f32>>,
	) {
		let Some(surface) = self.wl_surface_from_id(surface) else {return};
		self.seat.pointer_event(
			&surface,
			PointerEvent::Scroll {
				axis_continuous: scroll_distance,
				axis_discrete: scroll_steps,
			},
		)
	}

	fn keyboard_keymap(&self, surface: &SurfaceID, keymap_id: &str) {
		todo!()
	}
	fn keyboard_key(&self, surface: &SurfaceID, key: u32, state: bool) {
		let Some(surface) = self.wl_surface_from_id(surface) else {return};
		self.seat.keyboard_event(
			&surface,
			KeyboardEvent::Key {
				key,
				state: if state { 1 } else { 0 },
			},
		)
	}
}

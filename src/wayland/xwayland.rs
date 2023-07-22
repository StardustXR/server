use super::{panel_item::RecommendedState, seat::SeatData, state::WaylandState};
use crate::wayland::{
	panel_item::{Backend, PanelItem, X11Backend},
	surface::CoreSurface,
};
use color_eyre::eyre::Result;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use smithay::{
	reexports::{
		calloop::{EventLoop, LoopSignal},
		wayland_protocols::xdg::shell::server::xdg_toplevel,
		wayland_server::{Display, DisplayHandle, Resource, WEnum},
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
	pub fn create(
		wayland_display: Arc<Mutex<Display<WaylandState>>>,
		dh: &DisplayHandle,
	) -> Result<Self> {
		let dh = dh.clone();

		let (tx, rx) = oneshot::channel();

		tokio::task::spawn_blocking(move || {
			let mut event_loop: EventLoop<XWaylandHandler> = EventLoop::try_new()?;
			let (xwayland, connection) = XWayland::new(&dh);
			let handle = event_loop.handle();
			event_loop
				.handle()
				.insert_source(connection, move |event, _, handler| match event {
					XWaylandEvent::Ready {
						connection,
						client,
						client_fd: _,
						display: _,
					} => {
						handler.seat = Some(SeatData::new(&dh, client.id()));
						handler.wm =
							X11Wm::start_wm(handle.clone(), dh.clone(), connection, client).ok();
					}
					XWaylandEvent::Exited => (),
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
			let wayland_display_handle = wayland_display.lock().handle();
			let mut handler = XWaylandHandler {
				wayland_display,
				wayland_display_handle,
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
	wayland_display: Arc<Mutex<Display<WaylandState>>>,
	wayland_display_handle: DisplayHandle,
	wm: Option<X11Wm>,
	seat: Option<Arc<SeatData>>,
}
impl XWaylandHandler {
	fn panel_item(&self, window: &X11Surface) -> Option<Arc<PanelItem>> {
		compositor::with_states(&window.wl_surface()?, |s| {
			s.data_map.get::<Arc<PanelItem>>().cloned()
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
			&self.wayland_display,
			self.wayland_display.lock().handle(),
			&window.wl_surface().unwrap(),
			{
				let window = window.clone();
				move || {
					let Some(wl_surface) = window.wl_surface() else {return};
					let seat = seat.clone();
					window.user_data().insert_if_missing_threadsafe(|| {
						let (_node, panel_item) = PanelItem::create(
							wl_surface.clone(),
							Backend::X11(X11Backend {
								toplevel_parent: None,
								toplevel: window.clone(),
							}),
							wl_surface
								.client()
								.and_then(|c| c.get_credentials(&dh).ok()),
							seat,
						);
						panel_item
					});
				}
			},
			move |_| {
				let Some(panel_item) = window.user_data().get::<Arc<PanelItem>>() else {return};
				panel_item.commit_toplevel();
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
		panel_item.recommend_toplevel_state(RecommendedState::Move);
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
		panel_item.recommend_toplevel_state(RecommendedState::Resize(
			WEnum::Value(match resize_edge {
				ResizeEdge::Top => xdg_toplevel::ResizeEdge::Top,
				ResizeEdge::Bottom => xdg_toplevel::ResizeEdge::Bottom,
				ResizeEdge::Left => xdg_toplevel::ResizeEdge::Left,
				ResizeEdge::TopLeft => xdg_toplevel::ResizeEdge::TopLeft,
				ResizeEdge::BottomLeft => xdg_toplevel::ResizeEdge::BottomLeft,
				ResizeEdge::Right => xdg_toplevel::ResizeEdge::Right,
				ResizeEdge::TopRight => xdg_toplevel::ResizeEdge::TopRight,
				ResizeEdge::BottomRight => xdg_toplevel::ResizeEdge::BottomRight,
			})
			.into(),
		));
	}

	fn maximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
		let Some(panel_item) = self.panel_item(&window) else {return};
		panel_item.recommend_toplevel_state(RecommendedState::Maximize(true));
	}
	fn unmaximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
		let Some(panel_item) = self.panel_item(&window) else {return};
		panel_item.recommend_toplevel_state(RecommendedState::Maximize(false));
	}
	fn fullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
		let Some(panel_item) = self.panel_item(&window) else {return};
		panel_item.recommend_toplevel_state(RecommendedState::Fullscreen(true));
	}
	fn unfullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
		let Some(panel_item) = self.panel_item(&window) else {return};
		panel_item.recommend_toplevel_state(RecommendedState::Fullscreen(true));
	}
	fn minimize_request(&mut self, _xwm: XwmId, window: X11Surface) {
		let Some(panel_item) = self.panel_item(&window) else {return};
		panel_item.recommend_toplevel_state(RecommendedState::Minimize);
	}
}

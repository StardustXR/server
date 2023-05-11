use super::seat::SeatData;
use color_eyre::eyre::Result;
use once_cell::sync::OnceCell;
use smithay::{
	reexports::{
		calloop::{self, EventLoop, LoopSignal},
		wayland_server::DisplayHandle,
		x11rb::protocol::xproto::Window,
	},
	utils::{Logical, Rectangle},
	xwayland::{
		xwm::{Reorder, ResizeEdge, XwmId},
		X11Surface, X11Wm, XWayland, XWaylandEvent, XwmHandler,
	},
};
use std::{ffi::OsStr, iter::empty, sync::Arc, time::Duration};
use tokio::{sync::oneshot, task::JoinHandle};
use tracing::debug;

pub static DISPLAY: OnceCell<String> = OnceCell::new();

pub struct XWaylandState {
	pub display: u32,
	xwayland: XWayland,
	event_loop_signal: LoopSignal,
	event_loop_join: Option<JoinHandle<Result<(), calloop::Error>>>,
}
impl XWaylandState {
	pub fn create(dh: &DisplayHandle) -> Result<Self> {
		let dh = dh.clone();

		let (tx, rx) = oneshot::channel();

		let event_loop_join = tokio::task::spawn_blocking(move || {
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
				move |_| (),
			)?;
			let _ = tx.send(XWaylandState {
				display,
				xwayland,
				event_loop_signal: event_loop.get_signal(),
				event_loop_join: None,
			});
			let mut handler = XWaylandHandler::default();
			event_loop.run(Duration::from_secs(60 * 60), &mut handler, |_| ())
		});

		let mut state = rx.blocking_recv()?;
		state.event_loop_join.replace(event_loop_join);
		let _ = DISPLAY.set(format!(":{}", state.display));

		Ok(state)
	}
}
impl Drop for XWaylandState {
	fn drop(&mut self) {
		self.xwayland.shutdown();
		self.event_loop_signal.stop();
	}
}

#[derive(Default)]
struct XWaylandHandler {
	wm: Option<X11Wm>,
	seat: Option<Arc<SeatData>>,
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

	fn resize_request(
		&mut self,
		_xwm: XwmId,
		window: X11Surface,
		button: u32,
		resize_edge: ResizeEdge,
	) {
		debug!(?window, button, ?resize_edge, "X window requests resize");
	}

	fn move_request(&mut self, _xwm: XwmId, window: X11Surface, button: u32) {
		debug!(?window, button, "X window requests move");
	}
}

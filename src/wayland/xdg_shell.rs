use super::{
	seat::{CursorInfo, KeyboardEvent, PointerEvent, SeatData},
	state::{ClientState, WaylandState},
	surface::CoreSurface,
	SERIAL_COUNTER,
};
use crate::{
	nodes::{
		data::KEYMAPS,
		drawable::model::ModelPart,
		items::panel::{
			Backend, ChildInfo, Geometry, PanelItem, PanelItemInitData, SurfaceID, ToplevelInfo,
		},
	},
	wayland::seat::handle_cursor,
};
use color_eyre::eyre::{bail, eyre, Result};
use mint::Vector2;
use nanoid::nanoid;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use smithay::reexports::{
	wayland_protocols::xdg::shell::server::{
		xdg_popup::{self, XdgPopup},
		xdg_positioner::{self, Anchor, ConstraintAdjustment, Gravity, XdgPositioner},
		xdg_surface::{self, XdgSurface},
		xdg_toplevel::{self, ResizeEdge, XdgToplevel, EVT_WM_CAPABILITIES_SINCE},
		xdg_wm_base::{self, XdgWmBase},
	},
	wayland_server::{
		backend::{ClientId, ObjectId},
		protocol::wl_surface::WlSurface,
		Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
		Weak as WlWeak,
	},
};
use std::{
	fmt::Debug,
	sync::{Arc, Weak},
};
use tokio::sync::watch;
use tracing::{debug, warn};

impl GlobalDispatch<XdgWmBase, (), WaylandState> for WaylandState {
	fn bind(
		_state: &mut WaylandState,
		_handle: &DisplayHandle,
		_client: &Client,
		resource: New<XdgWmBase>,
		_global_data: &(),
		data_init: &mut DataInit<'_, WaylandState>,
	) {
		data_init.init(resource, ());
	}
}

impl Dispatch<XdgWmBase, (), WaylandState> for WaylandState {
	fn request(
		_state: &mut WaylandState,
		_client: &Client,
		_resource: &XdgWmBase,
		request: xdg_wm_base::Request,
		_data: &(),
		_dhandle: &DisplayHandle,
		data_init: &mut DataInit<'_, WaylandState>,
	) {
		match request {
			xdg_wm_base::Request::CreatePositioner { id } => {
				let positioner = data_init.init(id, Mutex::new(PositionerData::default()));
				debug!(?positioner, "Create XDG positioner");
			}
			xdg_wm_base::Request::GetXdgSurface { id, surface } => {
				let xdg_surface = data_init.init(id, Mutex::new(XdgSurfaceData::new(&surface)));
				debug!(?xdg_surface, "Create XDG surface");
			}
			xdg_wm_base::Request::Pong { serial } => {
				debug!(serial, "Client pong");
			}
			xdg_wm_base::Request::Destroy => {
				debug!("Destroy XDG WM base");
			}
			_ => unreachable!(),
		}
	}
}

#[derive(Debug, Clone, Copy)]
pub struct PositionerData {
	size: Vector2<u32>,
	anchor_rect_pos: Vector2<i32>,
	anchor_rect_size: Vector2<u32>,
	anchor: Anchor,
	gravity: Gravity,
	constraint_adjustment: ConstraintAdjustment,
	offset: Vector2<i32>,
	reactive: bool,
}
impl Default for PositionerData {
	fn default() -> Self {
		Self {
			size: Vector2::from([0; 2]),
			anchor_rect_pos: Vector2::from([0; 2]),
			anchor_rect_size: Vector2::from([0; 2]),
			anchor: Anchor::None,
			gravity: Gravity::None,
			constraint_adjustment: ConstraintAdjustment::None,
			offset: Vector2::from([0; 2]),
			reactive: false,
		}
	}
}

impl PositionerData {
	fn anchor_has_edge(&self, edge: Anchor) -> bool {
		match edge {
			Anchor::Top => {
				self.anchor == Anchor::Top
					|| self.anchor == Anchor::TopLeft
					|| self.anchor == Anchor::TopRight
			}
			Anchor::Bottom => {
				self.anchor == Anchor::Bottom
					|| self.anchor == Anchor::BottomLeft
					|| self.anchor == Anchor::BottomRight
			}
			Anchor::Left => {
				self.anchor == Anchor::Left
					|| self.anchor == Anchor::TopLeft
					|| self.anchor == Anchor::BottomLeft
			}
			Anchor::Right => {
				self.anchor == Anchor::Right
					|| self.anchor == Anchor::TopRight
					|| self.anchor == Anchor::BottomRight
			}
			_ => unreachable!(),
		}
	}

	fn gravity_has_edge(&self, edge: Gravity) -> bool {
		match edge {
			Gravity::Top => {
				self.gravity == Gravity::Top
					|| self.gravity == Gravity::TopLeft
					|| self.gravity == Gravity::TopRight
			}
			Gravity::Bottom => {
				self.gravity == Gravity::Bottom
					|| self.gravity == Gravity::BottomLeft
					|| self.gravity == Gravity::BottomRight
			}
			Gravity::Left => {
				self.gravity == Gravity::Left
					|| self.gravity == Gravity::TopLeft
					|| self.gravity == Gravity::BottomLeft
			}
			Gravity::Right => {
				self.gravity == Gravity::Right
					|| self.gravity == Gravity::TopRight
					|| self.gravity == Gravity::BottomRight
			}
			_ => unreachable!(),
		}
	}

	pub fn get_pos(&self) -> Vector2<i32> {
		let mut pos = self.offset;

		if self.anchor_has_edge(Anchor::Top) {
			pos.y += self.anchor_rect_pos.y;
		} else if self.anchor_has_edge(Anchor::Bottom) {
			pos.y += self.anchor_rect_pos.y + self.anchor_rect_size.y as i32;
		} else {
			pos.y += self.anchor_rect_pos.y + self.anchor_rect_size.y as i32 / 2;
		}

		if self.anchor_has_edge(Anchor::Left) {
			pos.x += self.anchor_rect_pos.x;
		} else if self.anchor_has_edge(Anchor::Right) {
			pos.x += self.anchor_rect_pos.x + self.anchor_rect_size.x as i32;
		} else {
			pos.x += self.anchor_rect_pos.x + self.anchor_rect_size.x as i32 / 2;
		}

		if self.gravity_has_edge(Gravity::Top) {
			pos.y -= self.size.y as i32;
		} else if !self.gravity_has_edge(Gravity::Bottom) {
			pos.y -= self.size.y as i32 / 2;
		}

		if self.gravity_has_edge(Gravity::Left) {
			pos.x -= self.size.x as i32;
		} else if !self.gravity_has_edge(Gravity::Right) {
			pos.x -= self.size.x as i32 / 2;
		}

		pos
	}
}
impl From<PositionerData> for Geometry {
	fn from(value: PositionerData) -> Self {
		Geometry {
			origin: value.get_pos(),
			size: value.size,
		}
	}
}

impl Dispatch<XdgPositioner, Mutex<PositionerData>, WaylandState> for WaylandState {
	fn request(
		_state: &mut WaylandState,
		_client: &Client,
		positioner: &XdgPositioner,
		request: xdg_positioner::Request,
		data: &Mutex<PositionerData>,
		_dhandle: &DisplayHandle,
		_data_init: &mut DataInit<'_, WaylandState>,
	) {
		match request {
			xdg_positioner::Request::SetSize { width, height } => {
				debug!(?positioner, width, height, "Set positioner size");
				data.lock().size = Vector2::from([width as u32, height as u32]);
			}
			xdg_positioner::Request::SetAnchorRect {
				x,
				y,
				width,
				height,
			} => {
				if width < 1 || height < 1 {
					positioner.post_error(
						xdg_positioner::Error::InvalidInput,
						"Invalid size for positioner's anchor rectangle.",
					);
					warn!(
						?positioner,
						width, height, "Invalid size for positioner's anchor rectangle"
					);
					return;
				}

				debug!(
					?positioner,
					x, y, width, height, "Set positioner anchor rectangle"
				);
				let mut data = data.lock();
				data.anchor_rect_pos = [x, y].into();
				data.anchor_rect_size = [width as u32, height as u32].into();
			}
			xdg_positioner::Request::SetAnchor { anchor } => {
				if let WEnum::Value(anchor) = anchor {
					debug!(?positioner, ?anchor, "Set positioner anchor");
					data.lock().anchor = anchor;
				}
			}
			xdg_positioner::Request::SetGravity { gravity } => {
				if let WEnum::Value(gravity) = gravity {
					debug!(?positioner, ?gravity, "Set positioner gravity");
					data.lock().gravity = gravity;
				}
			}
			xdg_positioner::Request::SetConstraintAdjustment {
				constraint_adjustment,
			} => {
				debug!(
					?positioner,
					constraint_adjustment, "Set positioner constraint adjustment"
				);
				data.lock().constraint_adjustment =
					ConstraintAdjustment::from_bits(constraint_adjustment).unwrap();
			}
			xdg_positioner::Request::SetOffset { x, y } => {
				debug!(?positioner, x, y, "Set positioner offset");
				data.lock().offset = [x, y].into();
			}
			xdg_positioner::Request::SetReactive => {
				debug!(?positioner, "Set positioner reactive");
				data.lock().reactive = true;
			}
			xdg_positioner::Request::SetParentSize {
				parent_width,
				parent_height,
			} => {
				debug!(
					?positioner,
					parent_width, parent_height, "Set positioner parent size"
				);
			}
			xdg_positioner::Request::SetParentConfigure { serial } => {
				debug!(?positioner, serial, "Set positioner parent size");
			}
			xdg_positioner::Request::Destroy => (),
			_ => unreachable!(),
		}
	}
}

pub struct XdgSurfaceData {
	wl_surface: WlWeak<WlSurface>,
	surface_id: SurfaceID,
	panel_item: Weak<PanelItem<XDGBackend>>,
	geometry: Option<Geometry>,
}
impl XdgSurfaceData {
	pub fn new(wl_surface: &WlSurface) -> Self {
		XdgSurfaceData {
			wl_surface: wl_surface.downgrade(),
			surface_id: SurfaceID::Toplevel,
			panel_item: Weak::new(),
			geometry: None,
		}
	}
	pub fn get(xdg_surface: &XdgSurface) -> Option<&Mutex<Self>> {
		xdg_surface.data::<Mutex<Self>>()
	}
	pub fn wl_surface(&self) -> Option<WlSurface> {
		self.wl_surface.upgrade().ok()
	}
	pub fn panel_item(&self) -> Option<Arc<PanelItem<XDGBackend>>> {
		self.panel_item.upgrade()
	}
}
// impl Clone for XdgSurfaceData {
// 	fn clone(&self) -> Self {
// 		Self {
// 			wl_surface: self.wl_surface.clone(),
// 			geometry: self.geometry.clone(),
// 			surface_type: Mutex::new(self.surface_type.lock().clone()),
// 		}
// 	}
// }
impl Dispatch<XdgSurface, Mutex<XdgSurfaceData>, WaylandState> for WaylandState {
	fn request(
		state: &mut WaylandState,
		client: &Client,
		xdg_surface: &XdgSurface,
		request: xdg_surface::Request,
		xdg_surface_data: &Mutex<XdgSurfaceData>,
		_dhandle: &DisplayHandle,
		data_init: &mut DataInit<'_, WaylandState>,
	) {
		match request {
			xdg_surface::Request::GetToplevel { id } => {
				let toplevel_state = Mutex::new(ToplevelData::new(xdg_surface));
				let toplevel = data_init.init(id, toplevel_state);
				debug!(?toplevel, ?xdg_surface, "Create XDG toplevel");

				if toplevel.version() >= EVT_WM_CAPABILITIES_SINCE {
					toplevel.wm_capabilities(vec![3]);
				}
				toplevel.configure(
					0,
					0,
					if toplevel.version() >= 2 {
						vec![1, 5, 6, 7, 8]
							.into_iter()
							.flat_map(u32::to_ne_bytes)
							.collect()
					} else {
						vec![]
					},
				);
				xdg_surface.configure(SERIAL_COUNTER.inc());

				let client_credentials = client.get_credentials(&state.display_handle).ok();
				let Some(seat_data) = client.get_data::<ClientState>().map(|s| s.seat.clone()) else {return};
				let Some(wl_surface) = xdg_surface_data.lock().wl_surface() else {return};
				CoreSurface::add_to(
					state.display_handle.clone(),
					&wl_surface,
					{
						let toplevel = toplevel.downgrade();
						move || {
							let toplevel = toplevel.upgrade().unwrap();
							let toplevel_data = ToplevelData::get(&toplevel);
							let Some(xdg_surface) = toplevel_data.lock().xdg_surface() else {return};
							let Some(xdg_surface_data) = XdgSurfaceData::get(&xdg_surface) else {return};

							xdg_surface_data.lock().surface_id = SurfaceID::Toplevel;
							let Some(backend) = XDGBackend::create(toplevel.clone(), seat_data.clone()) else {return};
							let panel_item = PanelItem::create(
								Box::new(backend),
								client_credentials.map(|c| c.pid),
							);
							xdg_surface_data.lock().panel_item = Arc::downgrade(&panel_item);
							handle_cursor(&panel_item, panel_item.backend.cursor.clone());
						}
					},
					{
						let toplevel = toplevel.downgrade();
						move |_| {
							let toplevel = toplevel.upgrade().unwrap();
							let toplevel_data = ToplevelData::get(&toplevel);
							let Some(panel_item) = toplevel_data.lock().panel_item() else {
								let Some(xdg_surface) = toplevel_data.lock().xdg_surface() else {return};
								// if the wayland toplevel isn't mapped, hammer it again with a configure until it cooperates
								toplevel.configure(
									0,
									0,
									if toplevel.version() >= 2 {
										vec![5, 6, 7, 8].into_iter().flat_map(u32::to_ne_bytes).collect()
									} else {
										vec![]
									},
								);
								xdg_surface.configure(SERIAL_COUNTER.inc());
								return
							};
							let Some(xdg_surface) = toplevel_data.lock().xdg_surface() else {return};
							let Some(xdg_surface_data) = XdgSurfaceData::get(&xdg_surface) else {return};
							let Some(wl_surface) = xdg_surface_data.lock().wl_surface() else {return};
							let Some(core_surface) = CoreSurface::from_wl_surface(&wl_surface) else {return};
							let Some(size) = core_surface.size() else {return};
							panel_item.toplevel_size_changed(size);
						}
					},
				);
			}
			xdg_surface::Request::GetPopup {
				id,
				parent,
				positioner,
			} => {
				let parent_clone = parent.clone().unwrap();
				let parent_data = parent_clone.data::<Mutex<XdgSurfaceData>>().unwrap().lock();
				// let positioner_data = positioner
				// 	.data::<Mutex<PositionerData>>()
				// 	.unwrap()
				// 	.lock()
				// 	.clone();
				// let parent = match &*parent_data {
				// 	XdgSurfaceType::Toplevel(_) => SurfaceID::Toplevel,
				// 	XdgSurfaceType::Popup(p) => {
				// 		SurfaceID::Popup(p.upgrade().unwrap().uid.clone())
				// 	}
				// 	XdgSurfaceType::Unknown => return,
				// };
				let uid = nanoid!();
				let popup_data = Mutex::new(PopupData::new(
					uid.clone(),
					xdg_surface,
					parent_data.surface_id.clone(),
					positioner,
				));
				let panel_item = parent_data.panel_item().unwrap();
				handle_cursor(
					&panel_item,
					panel_item
						.backend
						.seat
						.new_surface(&popup_data.lock().wl_surface().unwrap()),
				);
				let xdg_popup = data_init.init(id, popup_data);
				xdg_surface_data.lock().surface_id = SurfaceID::Child(uid);

				xdg_surface_data.lock().panel_item = Arc::downgrade(&panel_item);
				debug!(?xdg_popup, ?xdg_surface, "Create XDG popup");

				let xdg_surface = xdg_surface.downgrade();
				let xdg_popup = xdg_popup.downgrade();
				CoreSurface::add_to(
					state.display_handle.clone(),
					&xdg_surface_data.lock().wl_surface.upgrade().unwrap(),
					move || {
						let xdg_popup = xdg_popup.upgrade().unwrap();
						let Some(popup_data) = PopupData::get(&xdg_popup) else {return};
						let popup_data = popup_data.lock();
						panel_item
							.backend
							.new_popup(&panel_item, &xdg_popup, &*popup_data);
					},
					move |commit_count| {
						if commit_count == 0 {
							xdg_surface
								.upgrade()
								.unwrap()
								.configure(SERIAL_COUNTER.inc())
						}
					},
				);
			}
			xdg_surface::Request::SetWindowGeometry {
				x,
				y,
				width,
				height,
			} => {
				debug!(
					?xdg_surface,
					x, y, width, height, "Set XDG surface geometry"
				);
				let geometry = Geometry {
					origin: [x, y].into(),
					size: [width as u32, height as u32].into(),
				};
				xdg_surface_data.lock().geometry.replace(geometry);
			}
			xdg_surface::Request::AckConfigure { serial } => {
				debug!(?xdg_surface, serial, "Acknowledge XDG surface configure");
			}
			xdg_surface::Request::Destroy => {
				debug!(?xdg_surface, "Destroy XDG surface");
			}
			_ => unreachable!(),
		}
	}
}

#[derive(Debug, Clone)]
pub struct ToplevelData {
	xdg_surface: WlWeak<XdgSurface>,
	parent: Option<WlWeak<XdgToplevel>>,
	title: Option<String>,
	app_id: Option<String>,
	max_size: Option<Vector2<u32>>,
	min_size: Option<Vector2<u32>>,
}
impl ToplevelData {
	fn new(xdg_surface: &XdgSurface) -> Self {
		ToplevelData {
			xdg_surface: xdg_surface.downgrade(),
			parent: None,
			title: None,
			app_id: None,
			max_size: None,
			min_size: None,
		}
	}

	pub fn get(toplevel: &XdgToplevel) -> &Mutex<ToplevelData> {
		toplevel.data::<Mutex<ToplevelData>>().unwrap()
	}

	pub fn xdg_surface(&self) -> Option<XdgSurface> {
		self.xdg_surface.upgrade().ok()
	}
	fn panel_item(&self) -> Option<Arc<PanelItem<XDGBackend>>> {
		let xdg_surface = self.xdg_surface()?;
		let xdg_surface_data = XdgSurfaceData::get(&xdg_surface)?.lock();
		xdg_surface_data.panel_item()
	}
}
impl Drop for ToplevelData {
	fn drop(&mut self) {
		let Some(panel_item) = self.panel_item() else {return};
		panel_item.drop_toplevel();
	}
}
impl Dispatch<XdgToplevel, Mutex<ToplevelData>, WaylandState> for WaylandState {
	fn request(
		_state: &mut WaylandState,
		_client: &Client,
		xdg_toplevel: &XdgToplevel,
		request: xdg_toplevel::Request,
		data: &Mutex<ToplevelData>,
		_dhandle: &DisplayHandle,
		_data_init: &mut DataInit<'_, WaylandState>,
	) {
		match request {
			xdg_toplevel::Request::SetParent { parent } => {
				debug!(?xdg_toplevel, ?parent, "Set XDG Toplevel parent");
				data.lock().parent = parent.clone().map(|toplevel| toplevel.downgrade());
				let Some(panel_item) = data.lock().panel_item() else {return};
				if let Some(parent) = parent {
					panel_item.toplevel_parent_changed(
						&ToplevelData::get(&parent).lock().panel_item().unwrap().uid,
					);
				}
			}
			xdg_toplevel::Request::SetTitle { title } => {
				debug!(?xdg_toplevel, ?title, "Set XDG Toplevel title");
				data.lock().title = (!title.is_empty()).then_some(title.clone());

				let Some(panel_item) = data.lock().panel_item() else {return};
				panel_item.toplevel_title_changed(&title);
			}
			xdg_toplevel::Request::SetAppId { app_id } => {
				debug!(?xdg_toplevel, ?app_id, "Set XDG Toplevel app ID");
				data.lock().app_id = (!app_id.is_empty()).then_some(app_id.clone());

				let Some(panel_item) = data.lock().panel_item() else {return};
				panel_item.toplevel_app_id_changed(&app_id);
			}
			xdg_toplevel::Request::Move { seat, serial } => {
				debug!(?xdg_toplevel, ?seat, serial, "XDG Toplevel move request");
				let Some(panel_item) = data.lock().panel_item() else {return};
				panel_item.toplevel_move_request();
			}
			xdg_toplevel::Request::Resize {
				seat,
				serial,
				edges,
			} => {
				let WEnum::Value(edges) = edges else {return};
				debug!(
					?xdg_toplevel,
					?seat,
					serial,
					?edges,
					"XDG Toplevel resize request"
				);
				let Some(panel_item) = data.lock().panel_item() else {return};
				let (up, down, left, right) = match edges {
					ResizeEdge::Top => (true, false, false, false),
					ResizeEdge::Bottom => (false, true, false, false),
					ResizeEdge::Left => (false, false, true, false),
					ResizeEdge::TopLeft => (true, false, true, false),
					ResizeEdge::BottomLeft => (false, true, true, false),
					ResizeEdge::Right => (false, false, false, true),
					ResizeEdge::TopRight => (true, false, false, true),
					ResizeEdge::BottomRight => (false, true, false, true),
					_ => (false, false, false, false),
				};
				panel_item.toplevel_resize_request(up, down, left, right)
			}
			xdg_toplevel::Request::SetMaxSize { width, height } => {
				debug!(?xdg_toplevel, width, height, "Set XDG Toplevel max size");
				data.lock().max_size = (width > 1 || height > 1)
					.then_some(Vector2::from([width as u32, height as u32]));
			}
			xdg_toplevel::Request::SetMinSize { width, height } => {
				debug!(?xdg_toplevel, width, height, "Set XDG Toplevel min size");
				data.lock().min_size = (width > 1 || height > 1)
					.then_some(Vector2::from([width as u32, height as u32]));
			}
			xdg_toplevel::Request::SetFullscreen { output: _ } => {
				let Some(panel_item) = data.lock().panel_item() else {return};
				panel_item.backend.toplevel_state.lock().fullscreen = true;
				panel_item.backend.configure(None);
			}
			xdg_toplevel::Request::UnsetFullscreen => {
				let Some(panel_item) = data.lock().panel_item() else {return};
				panel_item.backend.toplevel_state.lock().fullscreen = false;
				panel_item.backend.configure(None);
			}
			xdg_toplevel::Request::Destroy => {
				debug!(?xdg_toplevel, "Destroy XDG Toplevel");
				let Some(panel_item) = data.lock().panel_item() else {return};
				panel_item.drop_toplevel();
			}
			_ => {}
		}
	}
}

#[derive(Clone)]
pub struct PopupData {
	pub uid: String,
	grabbed: bool,
	parent_id: SurfaceID,
	positioner: XdgPositioner,
	xdg_surface: WlWeak<XdgSurface>,
}
impl PopupData {
	fn new(
		uid: impl ToString,
		xdg_surface: &XdgSurface,
		parent_id: SurfaceID,
		positioner: XdgPositioner,
	) -> Self {
		PopupData {
			uid: uid.to_string(),
			grabbed: false,
			parent_id,
			positioner,
			xdg_surface: xdg_surface.downgrade(),
		}
	}
	pub fn get(popup: &XdgPopup) -> Option<&Mutex<Self>> {
		popup.data::<Mutex<Self>>()
	}
	pub fn xdg_surface(&self) -> Option<XdgSurface> {
		self.xdg_surface.upgrade().ok()
	}

	fn panel_item(&self) -> Option<Arc<PanelItem<XDGBackend>>> {
		XdgSurfaceData::get(&self.xdg_surface()?)?
			.lock()
			.panel_item()
	}
	// fn get_parent(&self) -> Option<XdgSurface> {
	// 	self.parent.as_ref()?.upgrade().ok()
	// }
	pub fn wl_surface(&self) -> Option<WlSurface> {
		XdgSurfaceData::get(&self.xdg_surface()?)?
			.lock()
			.wl_surface()
	}

	pub fn positioner_data(&self) -> Option<PositionerData> {
		Some(
			self.positioner
				.data::<Mutex<PositionerData>>()?
				.lock()
				.clone(),
		)
	}
}

impl Debug for PopupData {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("XdgPopupData")
			.field("uid", &self.uid)
			.field("positioner", &self.positioner)
			.field("xdg_surface", &self.xdg_surface)
			.finish()
	}
}
impl Dispatch<XdgPopup, Mutex<PopupData>, WaylandState> for WaylandState {
	fn request(
		_state: &mut WaylandState,
		_client: &Client,
		xdg_popup: &XdgPopup,
		request: xdg_popup::Request,
		data: &Mutex<PopupData>,
		_dhandle: &DisplayHandle,
		_data_init: &mut DataInit<'_, WaylandState>,
	) {
		match request {
			xdg_popup::Request::Grab { seat, serial } => {
				let mut data = data.lock();
				data.grabbed = true;
				debug!(?xdg_popup, ?seat, serial, "XDG popup grab");
				let Some(panel_item) = data.panel_item() else {return};
				panel_item.grab_keyboard(Some(SurfaceID::Child(data.uid.clone())));
			}
			xdg_popup::Request::Reposition { positioner, token } => {
				let mut data = data.lock();
				debug!(?xdg_popup, ?positioner, token, "XDG popup reposition");
				data.positioner = positioner;
				let Some(panel_item) = data.panel_item() else {return};

				panel_item.backend.reposition_popup(&panel_item, &*data)
			}
			xdg_popup::Request::Destroy => {
				let data = data.lock();
				debug!(?xdg_popup, "Destroy XDG popup");
				if data.grabbed {
					let Some(panel_item) = data.panel_item() else {return};
					panel_item.grab_keyboard(None);
				}
			}
			_ => unreachable!(),
		}
	}

	fn destroyed(
		_state: &mut WaylandState,
		_client: ClientId,
		_resource: ObjectId,
		data: &Mutex<PopupData>,
	) {
		let data = data.lock();
		let Some(panel_item) = data.panel_item() else {return};
		panel_item.backend.drop_popup(&panel_item, &data.uid);
	}
}

struct XdgToplevelState {
	fullscreen: bool,
	activated: bool,
}

pub struct XDGBackend {
	toplevel: WlWeak<XdgToplevel>,
	toplevel_wl_surface: WlWeak<WlSurface>,
	toplevel_state: Mutex<XdgToplevelState>,
	popups: Mutex<FxHashMap<String, WlWeak<XdgPopup>>>,
	cursor: watch::Receiver<Option<CursorInfo>>,
	seat: Arc<SeatData>,
	pointer_grab: Mutex<Option<SurfaceID>>,
	keyboard_grab: Mutex<Option<SurfaceID>>,
}
impl XDGBackend {
	pub fn create(toplevel: XdgToplevel, seat: Arc<SeatData>) -> Option<Self> {
		let toplevel_wl_surface =
			XdgSurfaceData::get(&ToplevelData::get(&toplevel).lock().xdg_surface()?)?
				.lock()
				.wl_surface()?
				.downgrade();

		let cursor = seat.new_surface(&toplevel_wl_surface.upgrade().ok()?);
		Some(XDGBackend {
			toplevel: toplevel.downgrade(),
			toplevel_wl_surface,
			toplevel_state: Mutex::new(XdgToplevelState {
				fullscreen: false,
				activated: false,
			}),
			popups: Mutex::new(FxHashMap::default()),
			cursor,
			seat,
			pointer_grab: Mutex::new(None),
			keyboard_grab: Mutex::new(None),
		})
	}
	fn wl_surface_from_id(&self, id: &SurfaceID) -> Option<WlSurface> {
		match id {
			SurfaceID::Cursor => self.cursor.borrow().as_ref()?.surface.upgrade().ok(),
			SurfaceID::Toplevel => self.toplevel_wl_surface(),
			SurfaceID::Child(popup) => {
				let popups = self.popups.lock();
				let popup = popups.get(popup)?.upgrade().ok()?;
				let wl_surface = PopupData::get(&popup)?.lock().wl_surface();
				wl_surface
			}
		}
	}
	fn toplevel(&self) -> Option<XdgToplevel> {
		self.toplevel.upgrade().ok()
	}
	fn toplevel_xdg_surface(&self) -> Option<XdgSurface> {
		let toplevel = self.toplevel()?;
		let data = ToplevelData::get(&toplevel).lock();
		data.xdg_surface()
	}
	fn toplevel_wl_surface(&self) -> Option<WlSurface> {
		self.toplevel_wl_surface.upgrade().ok()
	}

	fn configure(&self, size: Option<Vector2<u32>>) {
		let Ok(xdg_toplevel) = self.toplevel.upgrade() else {return};
		let Some(xdg_surface) = self.toplevel_xdg_surface() else {return};
		let Some(wl_surface) = self.toplevel_wl_surface() else {return};
		let Some(core_surface) = CoreSurface::from_wl_surface(&wl_surface) else {return};
		let Some(surface_size) = core_surface.size() else {return};

		xdg_toplevel.configure(
			size.unwrap_or(surface_size).x as i32,
			size.unwrap_or(surface_size).y as i32,
			self.states()
				.into_iter()
				.flat_map(|state| state.to_ne_bytes())
				.collect(),
		);
		xdg_surface.configure(SERIAL_COUNTER.inc());
		self.flush_client();
	}
	fn states(&self) -> Vec<u32> {
		let mut states = vec![1, 5, 6, 7, 8]; // maximized always and tiled
		let toplevel_state = self.toplevel_state.lock();
		if toplevel_state.fullscreen {
			states.push(2);
		}
		if toplevel_state.activated {
			states.push(4);
		}
		states
	}

	pub fn new_popup(
		&self,
		panel_item: &PanelItem<XDGBackend>,
		popup: &XdgPopup,
		data: &PopupData,
	) {
		self.popups
			.lock()
			.insert(data.uid.clone(), popup.downgrade());

		let Some(positioner_data) = data.positioner_data() else {return};
		panel_item.new_child(
			&data.uid,
			ChildInfo {
				parent: data.parent_id.clone(),
				geometry: positioner_data.into(),
			},
		)
	}
	pub fn reposition_popup(&self, panel_item: &PanelItem<XDGBackend>, popup_state: &PopupData) {
		let Some(positioner_data) = popup_state.positioner_data() else {return};
		panel_item.reposition_child(&popup_state.uid, positioner_data.into())
	}
	pub fn drop_popup(&self, panel_item: &PanelItem<XDGBackend>, uid: &str) {
		panel_item.drop_child(uid);
		let Some(popup) = self
				.popups
				.lock()
				.remove(uid) else {return};
		let Some(popup) = popup.upgrade().ok() else {return};
		let Some(popup) = popup.data::<Arc<PopupData>>().cloned() else {return};
		let Some(wl_surface) = popup.wl_surface() else {return};
		self.seat.drop_surface(&wl_surface);
	}

	fn child_data(&self) -> FxHashMap<String, ChildInfo> {
		FxHashMap::from_iter(self.popups.lock().values().filter_map(|v| {
			let popup = v.upgrade().ok()?;
			let data = PopupData::get(&popup)?;
			let data_lock = data.lock();
			Some((
				data_lock.uid.clone(),
				ChildInfo {
					parent: data_lock.parent_id.clone(),
					geometry: data_lock.positioner_data()?.into(),
				},
			))
		}))
	}

	fn flush_client(&self) {
		let Some(client) = self.toplevel_wl_surface().and_then(|s| s.client()) else {return};
		if let Some(client_state) = client.get_data::<ClientState>() {
			client_state.flush();
		}
	}
}
impl Drop for XDGBackend {
	fn drop(&mut self) {
		let Some(toplevel) = self.toplevel_wl_surface() else {return};
		self.seat.drop_surface(&toplevel);
		debug!("Dropped panel item gracefully");
	}
}
impl Backend for XDGBackend {
	fn start_data(&self) -> Result<PanelItemInitData> {
		let toplevel_data = self
			.toplevel()
			.map(|t| ToplevelData::get(&t).lock().clone())
			.ok_or_else(|| eyre!("Could not get toplevel"))?;

		let pointer_grab = self.pointer_grab.lock().clone();
		let keyboard_grab = self.keyboard_grab.lock().clone();

		let Some(wl_surface) = self.toplevel_wl_surface() else {bail!("Wayland surface not found")};
		let Some(core_surface) = CoreSurface::from_wl_surface(&wl_surface) else {bail!("Core surface not found")};
		let Some(size) = core_surface.size() else {bail!("Surface size not found")};

		let toplevel = ToplevelInfo {
			parent: toplevel_data
				.parent
				.as_ref()
				.and_then(|p| p.upgrade().ok())
				.and_then(|p| ToplevelData::get(&p).lock().panel_item())
				.map(|p| p.uid.clone()),
			title: toplevel_data.title.clone(),
			app_id: toplevel_data.app_id.clone(),
			size,
			min_size: toplevel_data.min_size.clone(),
			max_size: toplevel_data.max_size.clone(),
			logical_rectangle: XdgSurfaceData::get(&self.toplevel_xdg_surface().unwrap())
				.unwrap()
				.lock()
				.geometry
				.clone()
				.unwrap_or_else(|| Geometry {
					origin: [0, 0].into(),
					size,
				}),
		};

		Ok(PanelItemInitData {
			cursor: self.cursor.borrow().as_ref().and_then(|c| c.cursor_data()),
			toplevel,
			children: self.child_data(),
			pointer_grab,
			keyboard_grab,
		})
	}

	fn apply_surface_material(&self, surface: SurfaceID, model_part: &Arc<ModelPart>) {
		let Some(wl_surface) = self.wl_surface_from_id(&surface) else {return};
		let Some(core_surface) = CoreSurface::from_wl_surface(&wl_surface) else {return};

		core_surface.apply_material(model_part);
	}

	fn close_toplevel(&self) {
		let Ok(xdg_toplevel) = self.toplevel.upgrade() else {return};
		xdg_toplevel.close();
	}
	fn auto_size_toplevel(&self) {
		self.configure(Some([0, 0].into()));
	}
	fn set_toplevel_size(&self, size: Vector2<u32>) {
		self.configure(Some(size));
	}
	fn set_toplevel_focused_visuals(&self, focused: bool) {
		self.toplevel_state.lock().activated = focused;
		self.configure(None);
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

	fn keyboard_keys(&self, surface: &SurfaceID, keymap_id: &str, keys: Vec<i32>) {
		let Some(surface) = self.wl_surface_from_id(surface) else {return};
		let keymaps = KEYMAPS.lock();
		let Some(keymap) = keymaps.get(keymap_id).cloned() else {return};
		if self.seat.set_keymap(keymap, vec![surface.clone()]).is_err() {
			return;
		}
		for key in keys {
			self.seat.keyboard_event(
				&surface,
				KeyboardEvent::Key {
					key: key.abs() as u32,
					state: if key < 0 { 1 } else { 0 },
				},
			);
		}
	}
}

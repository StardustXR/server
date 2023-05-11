use crate::{
	nodes::Node,
	wayland::panel_item::{Backend, WaylandBackend},
};

use super::{
	panel_item::{PanelItem, RecommendedState, SurfaceID},
	state::WaylandState,
	surface::CoreSurface,
	SERIAL_COUNTER,
};
use mint::Vector2;
use nanoid::nanoid;
use parking_lot::Mutex;
use serde::{ser::SerializeSeq, Serialize, Serializer};
use smithay::reexports::{
	wayland_protocols::xdg::shell::server::{
		xdg_popup::{self, XdgPopup},
		xdg_positioner::{self, Anchor, ConstraintAdjustment, Gravity, XdgPositioner},
		xdg_surface::{self, XdgSurface},
		xdg_toplevel::{self, XdgToplevel, EVT_WM_CAPABILITIES_SINCE},
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

#[derive(Debug, Serialize, Clone, Copy)]
pub struct PositionerData {
	size: Vector2<u32>,
	anchor_rect_pos: Vector2<i32>,
	anchor_rect_size: Vector2<u32>,
	anchor: u32,
	gravity: u32,
	constraint_adjustment: u32,
	offset: Vector2<i32>,
	reactive: bool,
}
impl Default for PositionerData {
	fn default() -> Self {
		Self {
			size: Vector2::from([0; 2]),
			anchor_rect_pos: Vector2::from([0; 2]),
			anchor_rect_size: Vector2::from([0; 2]),
			anchor: Anchor::None as u32,
			gravity: Gravity::None as u32,
			constraint_adjustment: ConstraintAdjustment::None.bits(),
			offset: Vector2::from([0; 2]),
			reactive: false,
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
					data.lock().anchor = anchor as u32;
				}
			}
			xdg_positioner::Request::SetGravity { gravity } => {
				if let WEnum::Value(gravity) = gravity {
					debug!(?positioner, ?gravity, "Set positioner gravity");
					data.lock().gravity = gravity as u32;
				}
			}
			xdg_positioner::Request::SetConstraintAdjustment {
				constraint_adjustment,
			} => {
				debug!(
					?positioner,
					constraint_adjustment, "Set positioner constraint adjustment"
				);
				data.lock().constraint_adjustment = constraint_adjustment;
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

#[derive(Debug, Serialize, Clone, Copy)]
pub struct Geometry {
	pub origin: Vector2<i32>,
	pub size: Vector2<u32>,
}
impl Default for Geometry {
	fn default() -> Self {
		Self {
			origin: Vector2::from([0; 2]),
			size: Vector2::from([0; 2]),
		}
	}
}

pub struct XdgSurfaceData {
	wl_surface: WlWeak<WlSurface>,
	surface_id: SurfaceID,
	panel_item: Weak<PanelItem>,
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
	pub fn get(xdg_surface: &XdgSurface) -> &Mutex<Self> {
		xdg_surface.data::<Mutex<Self>>().unwrap()
	}
	pub fn wl_surface(&self) -> WlSurface {
		self.wl_surface.upgrade().unwrap()
	}
	pub fn panel_item(&self) -> Option<Arc<PanelItem>> {
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
					toplevel.wm_capabilities(vec![2, 3, 4]);
				}
				toplevel.configure(
					0,
					0,
					if toplevel.version() >= 2 {
						vec![5, 6, 7, 8]
							.into_iter()
							.flat_map(u32::to_ne_bytes)
							.collect()
					} else {
						vec![]
					},
				);
				xdg_surface.configure(SERIAL_COUNTER.inc());

				let client_credentials = client.get_credentials(&state.display_handle).ok();
				let seat_data = state.seats.get(&client.id()).unwrap().clone();
				CoreSurface::add_to(
					&state.display,
					state.display_handle.clone(),
					&xdg_surface_data.lock().wl_surface(),
					{
						let toplevel = toplevel.downgrade();
						move || {
							let toplevel = toplevel.upgrade().unwrap();
							let toplevel_data = ToplevelData::get(&toplevel);
							let xdg_surface = toplevel_data.lock().xdg_surface();
							let xdg_surface_data = XdgSurfaceData::get(&xdg_surface);
							let wl_surface = xdg_surface_data.lock().wl_surface();

							xdg_surface_data.lock().surface_id = SurfaceID::Toplevel;
							let toplevel = toplevel_weak.upgrade().unwrap();
							let (node, panel_item) = PanelItem::create(
								wl_surface.clone(),
								Backend::Wayland(WaylandBackend::create(toplevel)),
								client_credentials,
								seat_data.clone(),
							);
							toplevel_data.lock().panel_item_node.replace(node);
							xdg_surface_data.lock().panel_item = Arc::downgrade(&panel_item);
						}
					},
					{
						let toplevel = toplevel.downgrade();
						move |_| {
							let toplevel = toplevel.upgrade().unwrap();
							let toplevel_data = ToplevelData::get(&toplevel);
							let Some(panel_item) = toplevel_data.lock().panel_item() else {
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
								let xdg_surface = toplevel_data.lock().xdg_surface();
								xdg_surface.configure(SERIAL_COUNTER.inc());
								return
							};
							panel_item.commit_toplevel();
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
				let xdg_popup = data_init.init(id, popup_data);
				xdg_surface_data.lock().surface_id = SurfaceID::Popup(uid);
				let panel_item = parent_data.panel_item().unwrap();
				xdg_surface_data.lock().panel_item = Arc::downgrade(&panel_item);

				panel_item.seat_data.new_surface(
					&xdg_surface_data.lock().wl_surface(),
					Arc::downgrade(&panel_item),
				);
				debug!(?xdg_popup, ?xdg_surface, "Create XDG popup");

				let xdg_surface = xdg_surface.downgrade();
				let xdg_popup = xdg_popup.downgrade();
				CoreSurface::add_to(
					&state.display,
					state.display_handle.clone(),
					&xdg_surface_data.lock().wl_surface.upgrade().unwrap(),
					move || {
						let xdg_popup = xdg_popup.upgrade().unwrap();
						let popup_data = PopupData::get(&xdg_popup);
						let popup_data = popup_data.lock();
						// panel_item.commit_popup(popup_data);
						panel_item.new_popup(&xdg_popup, &*popup_data);
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

fn serde_error<S: Serializer>(msg: &str) -> Result<S::Ok, S::Error> {
	Err(serde::ser::Error::custom(msg))
}

#[derive(Debug, Clone)]
pub struct ToplevelData {
	panel_item_node: Option<Arc<Node>>,
	xdg_surface: WlWeak<XdgSurface>,
	parent: Option<WlWeak<XdgToplevel>>,
	title: Option<String>,
	app_id: Option<String>,
	max_size: Option<Vector2<u32>>,
	min_size: Option<Vector2<u32>>,
	states: Vec<u32>,
}
impl ToplevelData {
	fn new(xdg_surface: &XdgSurface) -> Self {
		ToplevelData {
			panel_item_node: None,
			xdg_surface: xdg_surface.downgrade(),
			parent: None,
			title: None,
			app_id: None,
			max_size: None,
			min_size: None,
			states: Vec::new(),
		}
	}

	pub fn get(toplevel: &XdgToplevel) -> &Mutex<ToplevelData> {
		toplevel.data::<Mutex<ToplevelData>>().unwrap()
	}

	pub fn xdg_surface(&self) -> XdgSurface {
		self.xdg_surface.upgrade().unwrap()
	}
	fn panel_item(&self) -> Option<Arc<PanelItem>> {
		let xdg_surface = self.xdg_surface();
		let xdg_surface_data = XdgSurfaceData::get(&xdg_surface).lock();
		xdg_surface_data.panel_item()
	}
}
impl Serialize for ToplevelData {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		let xdg_surface = self.xdg_surface();
		let xdg_surface_data = XdgSurfaceData::get(&xdg_surface).lock();
		let geometry = xdg_surface_data.geometry.clone();
		let wl_surface = xdg_surface_data.wl_surface();
		let Some(core_surface) = CoreSurface::from_wl_surface(&wl_surface) else {return serde_error::<S>("Core surface not found")};
		let Some(size) = core_surface.size() else {return serializer.serialize_none()};
		let geometry = geometry.unwrap_or_else(|| Geometry {
			origin: [0; 2].into(),
			size,
		});

		let mut seq = serializer.serialize_seq(None)?;
		// Parent UID
		seq.serialize_element(&self.parent.as_ref().and_then(|p| {
			Some(
				ToplevelData::get(&p.upgrade().ok()?)
					.lock()
					.panel_item()?
					.uid
					.clone(),
			)
		}))?;
		seq.serialize_element(&self.title)?;
		seq.serialize_element(&self.app_id)?;
		seq.serialize_element(&size)?;
		seq.serialize_element(&self.min_size)?;
		seq.serialize_element(&self.max_size)?;
		seq.serialize_element(&geometry)?;
		seq.serialize_element(&self.states)?;
		seq.end()
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
				data.lock().parent = parent.map(|toplevel| toplevel.downgrade());
			}
			xdg_toplevel::Request::SetTitle { title } => {
				debug!(?xdg_toplevel, ?title, "Set XDG Toplevel title");
				data.lock().title = (!title.is_empty()).then_some(title);
			}
			xdg_toplevel::Request::SetAppId { app_id } => {
				debug!(?xdg_toplevel, ?app_id, "Set XDG Toplevel app ID");
				data.lock().app_id = (!app_id.is_empty()).then_some(app_id);
			}
			xdg_toplevel::Request::ShowWindowMenu { seat, serial, x, y } => {
				debug!(
					?xdg_toplevel,
					?seat,
					serial,
					x,
					y,
					"Show XDG Toplevel window menu"
				);
			}
			xdg_toplevel::Request::Move { seat, serial } => {
				debug!(?xdg_toplevel, ?seat, serial, "XDG Toplevel move request");
				let Some(panel_item) = data.lock().panel_item() else { return };
				panel_item.recommend_toplevel_state(RecommendedState::Move);
			}
			xdg_toplevel::Request::Resize {
				seat,
				serial,
				edges,
			} => {
				let WEnum::Value(edges) = edges else { return };
				debug!(
					?xdg_toplevel,
					?seat,
					serial,
					?edges,
					"XDG Toplevel resize request"
				);
				let Some(panel_item) = data.lock().panel_item() else { return };
				panel_item.recommend_toplevel_state(RecommendedState::Resize(edges as u32));
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
			xdg_toplevel::Request::SetMaximized => {
				let Some(panel_item) = data.lock().panel_item() else { return };
				panel_item.recommend_toplevel_state(RecommendedState::Maximize(true));
			}
			xdg_toplevel::Request::UnsetMaximized => {
				let Some(panel_item) = data.lock().panel_item() else { return };
				panel_item.recommend_toplevel_state(RecommendedState::Maximize(false));
			}
			xdg_toplevel::Request::SetFullscreen { output: _ } => {
				let Some(panel_item) = data.lock().panel_item() else { return };
				panel_item.recommend_toplevel_state(RecommendedState::Fullscreen(true));
			}
			xdg_toplevel::Request::UnsetFullscreen => {
				let Some(panel_item) = data.lock().panel_item() else { return };
				panel_item.recommend_toplevel_state(RecommendedState::Fullscreen(true));
			}
			xdg_toplevel::Request::SetMinimized => {
				let Some(panel_item) = data.lock().panel_item() else { return };
				panel_item.recommend_toplevel_state(RecommendedState::Minimize);
			}
			xdg_toplevel::Request::Destroy => {
				debug!(?xdg_toplevel, "Destroy XDG Toplevel");
				let Some(panel_item) = data.lock().panel_item() else { return };
				panel_item.on_drop();
			}
			_ => unreachable!(),
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
	pub fn get(popup: &XdgPopup) -> &Mutex<Self> {
		popup.data::<Mutex<Self>>().unwrap()
	}
	pub fn xdg_surface(&self) -> XdgSurface {
		self.xdg_surface.upgrade().unwrap()
	}

	fn panel_item(&self) -> Option<Arc<PanelItem>> {
		XdgSurfaceData::get(&self.xdg_surface()).lock().panel_item()
	}
	// fn get_parent(&self) -> Option<XdgSurface> {
	// 	self.parent.as_ref()?.upgrade().ok()
	// }
	pub fn wl_surface(&self) -> WlSurface {
		XdgSurfaceData::get(&self.xdg_surface()).lock().wl_surface()
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

impl Serialize for PopupData {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		let Some(positioner_data) = self.positioner_data() else {return serde_error::<S>("Positioner not found")};
		let mut seq = serializer.serialize_seq(None)?;
		seq.serialize_element(&self.uid)?;
		seq.serialize_element(&self.parent_id)?;
		seq.serialize_element(&positioner_data)?;
		seq.end()
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
				panel_item.grab_keyboard(Some(SurfaceID::Popup(data.uid.clone())));
			}
			xdg_popup::Request::Reposition { positioner, token } => {
				let mut data = data.lock();
				debug!(?xdg_popup, ?positioner, token, "XDG popup reposition");
				data.positioner = positioner;
				let Some(panel_item) = data.panel_item() else {return};

				if let Backend::Wayland(w) = &panel_item.backend {
					w.reposition_popup(&panel_item, &*data)
				}
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

		if let Backend::Wayland(w) = &panel_item.backend {
			w.drop_popup(&panel_item, &data.uid);
		}
	}
}

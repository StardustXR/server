use crate::{
	nodes::items::panel::{Geometry, PanelItem, SurfaceID},
	wayland::{
		seat::handle_cursor,
		state::{ClientState, WaylandState},
		surface::CoreSurface,
		utils,
		xdg_shell::{popup::PopupData, toplevel::ToplevelData, XdgBackend},
		SERIAL_COUNTER,
	},
};
use nanoid::nanoid;
use parking_lot::Mutex;
use smithay::reexports::{
	wayland_protocols::xdg::shell::server::{
		xdg_surface::{self, XdgSurface},
		xdg_toplevel::{XdgToplevel, EVT_WM_CAPABILITIES_SINCE},
	},
	wayland_server::{
		protocol::wl_surface::WlSurface, Client, DataInit, Dispatch, DisplayHandle, Resource,
		Weak as WlWeak,
	},
};
use std::sync::{Arc, Weak};
use tracing::{debug, error};

#[derive(Debug)]
pub struct XdgSurfaceData {
	pub wl_surface: WlWeak<WlSurface>,
	pub xdg_surface: XdgSurface,
	pub geometry: Mutex<Option<Geometry>>,
}

impl Dispatch<XdgSurface, WlWeak<WlSurface>, WaylandState> for WaylandState {
	fn request(
		state: &mut WaylandState,
		client: &Client,
		xdg_surface: &XdgSurface,
		request: xdg_surface::Request,
		wl_surface_resource: &WlWeak<WlSurface>,
		_dhandle: &DisplayHandle,
		data_init: &mut DataInit<'_, WaylandState>,
	) {
		let Ok(wl_surface) = wl_surface_resource.upgrade() else {
			error!("Couldn't get the wayland surface of the xdg surface");
			return;
		};
		let Some(xdg_surface_data) = utils::get_data::<XdgSurfaceData>(&wl_surface) else {
			error!("Couldn't get the XdgSurface");
			return;
		};
		match request {
			xdg_surface::Request::GetToplevel { id } => {
				let toplevel = data_init.init(id, wl_surface_resource.clone());
				utils::insert_data(&wl_surface, SurfaceID::Toplevel);
				utils::insert_data(&wl_surface, toplevel.clone());
				utils::insert_data(&wl_surface, ToplevelData::new(&wl_surface));
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
				let Some(seat_data) = client.get_data::<ClientState>().map(|s| s.seat.clone())
				else {
					return;
				};

				let xdg_surface = xdg_surface.clone();
				CoreSurface::add_to(
					state.display_handle.clone(),
					&wl_surface,
					{
						let wl_surface_resource = wl_surface_resource.clone();
						move || {
							let wl_surface = wl_surface_resource.upgrade().unwrap();

							let backend = XdgBackend::create(
								wl_surface.clone(),
								toplevel.clone(),
								seat_data.clone(),
							);
							let (node, panel_item) = PanelItem::create(
								Box::new(backend),
								client_credentials.map(|c| c.pid),
							);
							utils::insert_data(&wl_surface, Arc::downgrade(&panel_item));
							utils::insert_data_raw(&wl_surface, node);
							handle_cursor(&panel_item, panel_item.backend.cursor.clone());
						}
					},
					{
						let wl_surface_resource = wl_surface_resource.clone();
						move |_| {
							let wl_surface = wl_surface_resource.upgrade().unwrap();

							let Some(panel_item) =
								utils::get_data::<PanelItem<XdgBackend>>(&wl_surface)
							else {
								let Some(toplevel) = utils::get_data::<XdgToplevel>(&wl_surface)
								else {
									return;
								};
								// if the wayland toplevel isn't mapped, hammer it again with a configure until it cooperates
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
								return;
							};
							let Some(core_surface) = CoreSurface::from_wl_surface(&wl_surface)
							else {
								return;
							};
							let Some(size) = core_surface.size() else {
								return;
							};
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
				let Some(parent) = parent else { return };
				let Some(parent_wl_surface) = parent
					.data::<WlWeak<WlSurface>>()
					.map(WlWeak::upgrade)
					.map(Result::ok)
					.flatten()
				else {
					return;
				};
				let Some(panel_item) =
					utils::get_data::<Weak<PanelItem<XdgBackend>>>(&parent_wl_surface)
						.as_deref()
						.and_then(Weak::upgrade)
				else {
					return;
				};

				let uid = nanoid!();
				let popup_data = PopupData::new(
					uid.clone(),
					parent_wl_surface.clone(),
					&panel_item,
					positioner,
				);
				handle_cursor(
					&panel_item,
					panel_item.backend.seat.new_surface(&wl_surface),
				);
				let xdg_popup = data_init.init(id, wl_surface.downgrade());
				utils::insert_data(&wl_surface, SurfaceID::Child(uid));
				utils::insert_data(&wl_surface, Arc::downgrade(&panel_item));
				utils::insert_data(&wl_surface, popup_data);
				utils::insert_data(&wl_surface, xdg_popup.clone());
				debug!(?xdg_popup, ?xdg_surface, "Create XDG popup");

				let xdg_surface = xdg_surface.downgrade();
				let popup_wl_surface = wl_surface.downgrade();
				CoreSurface::add_to(
					state.display_handle.clone(),
					&wl_surface,
					move || {
						let Ok(wl_surface) = popup_wl_surface.upgrade() else {
							return;
						};
						let Some(popup_data) = utils::get_data::<PopupData>(&wl_surface) else {
							return;
						};
						panel_item
							.backend
							.new_popup(&panel_item, &wl_surface, &*popup_data);
					},
					move |commit_count| {
						if commit_count == 0 {
							if let Ok(xdg_surface) = xdg_surface.upgrade() {
								xdg_surface.configure(SERIAL_COUNTER.inc())
							}
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
				xdg_surface_data.geometry.lock().replace(geometry);
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

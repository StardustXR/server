use super::{popup::Popup, positioner::Positioner, toplevel::MappedInner};
use crate::wayland::util::ClientExt;
use crate::wayland::{
	core::surface::SurfaceRole,
	display::Display,
	xdg::{toplevel::Toplevel, wm_base::XdgSurfaceRole},
};
use std::sync::{Arc, OnceLock, Weak};
pub use waynest::server::protocol::stable::xdg_shell::xdg_surface::*;
use waynest::{
	server::{Client, Dispatcher, Result},
	wire::ObjectId,
};

#[derive(Debug, Dispatcher)]
pub struct Surface {
	id: ObjectId,
	version: u32,
	wl_surface: Weak<crate::wayland::core::surface::Surface>,
	configured: Arc<std::sync::atomic::AtomicBool>,
}
impl Surface {
	pub fn new(
		id: ObjectId,
		version: u32,
		wl_surface: Arc<crate::wayland::core::surface::Surface>,
	) -> Self {
		Self {
			id,
			version,
			wl_surface: Arc::downgrade(&wl_surface),
			configured: Arc::new(std::sync::atomic::AtomicBool::new(false)),
		}
	}

	pub fn wl_surface(&self) -> Arc<crate::wayland::core::surface::Surface> {
		// We can safely unwrap as the surface must exist for the lifetime of the xdg_surface
		self.wl_surface
			.upgrade()
			.expect("Surface was dropped before xdg_surface")
	}

	pub async fn reconfigure(&self, client: &mut Client) -> Result<()> {
		let serial = client.next_event_serial();
		self.configure(client, self.id, serial).await
	}
}

impl XdgSurface for Surface {
	/// https://wayland.app/protocols/xdg-shell#xdg_surface:request:destroy
	async fn destroy(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		Ok(())
	}

	/// https://wayland.app/protocols/xdg-shell#xdg_surface:request:get_toplevel
	async fn get_toplevel(
		&self,
		client: &mut Client,
		sender_id: ObjectId,
		toplevel_id: ObjectId,
	) -> Result<()> {
		let surface = self.wl_surface();
		let toplevel = client.insert(
			toplevel_id,
			Toplevel::new(
				toplevel_id,
				surface.clone(),
				client.get::<Self>(sender_id).unwrap(),
			),
		);
		// Set up the XDG role if not already done
		if surface.role.get().is_none() {
			let xdg_role = SurfaceRole::Xdg(OnceLock::new());

			if surface.role.set(xdg_role).is_err() {
				return client
					.protocol_error(
						sender_id,
						toplevel_id,
						1, // XDG_WM_BASE_ERROR_ROLE
						"Failed to set surface role (race condition)".to_string(),
					)
					.await;
			}
		}

		// Check if the surface already has an XDG role
		let surface_role = surface.role.get().unwrap();

		// Now check if this is an XDG surface and set the sub-role
		if let SurfaceRole::Xdg(role) = surface_role {
			if role.get().is_some() {
				return client
					.protocol_error(
						sender_id,
						toplevel_id,
						1, // XDG_WM_BASE_ERROR_ROLE
						"XDG surface already has a sub-role".to_string(),
					)
					.await;
			}

			if role
				.set(XdgSurfaceRole::Toplevel(Arc::downgrade(&toplevel)))
				.is_err()
			{
				return client
					.protocol_error(
						sender_id,
						toplevel_id,
						1, // XDG_WM_BASE_ERROR_ROLE
						"Failed to set XDG sub-role (race condition)".to_string(),
					)
					.await;
			}
		} else {
			return client
				.protocol_error(
					sender_id,
					toplevel_id,
					1, // XDG_WM_BASE_ERROR_ROLE
					"Surface has a non-XDG role".to_string(),
				)
				.await;
		}

		toplevel.reconfigure(client).await?;

		let pid = client.get::<Display>(ObjectId::DISPLAY).unwrap().pid;
		let configured = self.configured.clone();
		surface.add_commit_handler(move |surface, state| {
			let Some(role_ref) = surface.role.get() else {
				return true;
			};

			let SurfaceRole::Xdg(role) = role_ref else {
				return true;
			};

			let Some(XdgSurfaceRole::Toplevel(toplevel)) = role.get() else {
				return true;
			};
			let Some(toplevel) = toplevel.upgrade() else {
				return true;
			};

			// Only proceed if configured and has valid buffer
			let has_valid_buffer = state
				.buffer
				.as_ref()
				.is_some_and(|b| b.buffer.size().x > 0 && b.buffer.size().y > 0);

			let mut mapped_lock = toplevel.mapped.lock();
			if mapped_lock.is_none()
				&& configured.load(std::sync::atomic::Ordering::SeqCst)
				&& has_valid_buffer
			{
				mapped_lock.replace(MappedInner::create(toplevel.clone(), pid));
				return false;
			}
			true
		});

		Ok(())
	}

	/// https://wayland.app/protocols/xdg-shell#xdg_surface:request:get_popup
	async fn get_popup(
		&self,
		client: &mut Client,
		sender_id: ObjectId,
		popup_id: ObjectId,
		parent: Option<ObjectId>,
		positioner: ObjectId,
	) -> Result<()> {
		let Some(parent) = parent else {
			return client
				.protocol_error(
					sender_id,
					popup_id,
					3, // INVALID_POPUP_PARENT
					"Parent surface does not have an XDG role".to_string(),
				)
				.await;
		};
		let parent = client.get::<Surface>(parent).unwrap();
		let panel_item = match parent.wl_surface().role.get().unwrap() {
			SurfaceRole::Xdg(role) => match role.get().unwrap() {
				XdgSurfaceRole::Toplevel(toplevel) => {
					if let Some(toplevel) = toplevel.upgrade() {
						let toplevel_lock = toplevel.mapped.lock();
						toplevel_lock.as_ref().unwrap()._panel_item.clone()
					} else {
						return client
							.protocol_error(
								sender_id,
								popup_id,
								3, // INVALID_POPUP_PARENT
								"Parent surface does not have an XDG role".to_string(),
							)
							.await;
					}
				}
				XdgSurfaceRole::Popup(popup) => {
					if let Some(popup) = popup.upgrade() {
						popup.panel_item.upgrade().unwrap()
					} else {
						return client
							.protocol_error(
								sender_id,
								popup_id,
								3, // INVALID_POPUP_PARENT
								"Parent surface does not have an XDG role".to_string(),
							)
							.await;
					}
				}
			},
			_ => {
				return client
					.protocol_error(
						sender_id,
						popup_id,
						1, // XDG_WM_BASE_ERROR_ROLE
						"Parent surface does not have an XDG role".to_string(),
					)
					.await;
			}
		};
		let positioner = client.get::<Positioner>(positioner).unwrap();

		let surface = client.get::<Surface>(self.id).unwrap();

		let popup = client.insert(
			popup_id,
			Popup::new(
				popup_id,
				self.version,
				parent,
				&panel_item,
				&surface,
				&positioner,
			),
		);

		// Set up the XDG role if not already done
		let wl_surface = self.wl_surface();
		if wl_surface.role.get().is_none() {
			let xdg_role = SurfaceRole::Xdg(OnceLock::new());

			if wl_surface.role.set(xdg_role).is_err() {
				return client
					.protocol_error(
						sender_id,
						popup_id,
						1, // XDG_WM_BASE_ERROR_ROLE
						"Failed to set surface role (race condition)".to_string(),
					)
					.await;
			}
		}

		// Now check if this is an XDG surface and set the sub-role
		match wl_surface.role.get().unwrap() {
			SurfaceRole::Xdg(role) => {
				if role.get().is_some() {
					return client
						.protocol_error(
							sender_id,
							popup_id,
							1, // XDG_WM_BASE_ERROR_ROLE
							"XDG surface already has a sub-role".to_string(),
						)
						.await;
				}

				if role
					.set(XdgSurfaceRole::Popup(Arc::downgrade(&popup)))
					.is_err()
				{
					return client
						.protocol_error(
							sender_id,
							popup_id,
							1, // XDG_WM_BASE_ERROR_ROLE
							"Failed to set XDG sub-role (race condition)".to_string(),
						)
						.await;
				}
			}
			_ => {
				return client
					.protocol_error(
						sender_id,
						popup_id,
						1, // XDG_WM_BASE_ERROR_ROLE
						"Surface has a non-XDG role".to_string(),
					)
					.await;
			}
		}

		let serial = client.next_event_serial();
		self.configure(client, sender_id, serial).await?;

		// let pid = client.get::<Display>(ObjectId::DISPLAY).unwrap().pid;
		// let configured = self.configured.clone();
		// surface.add_commit_handler(move |surface, state| {
		// 	let Some(SurfaceRole::XDGPopup(popup)) = &mut *surface.role.lock() else {
		// 		return true;
		// 	};

		// 	// Only proceed if configured and has valid buffer
		// 	let has_valid_buffer = state
		// 		.buffer
		// 		.as_ref()
		// 		.is_some_and(|b| b.buffer.size().x > 0 && b.buffer.size().y > 0);

		// 	let mut mapped_lock = popup.mapped.lock();
		// 	if mapped_lock.is_none()
		// 		&& configured.load(std::sync::atomic::Ordering::SeqCst)
		// 		&& has_valid_buffer
		// 	{
		// 		mapped_lock.replace(Mapped::create(popup.clone(), pid));
		// 		return false;
		// 	}
		// 	true
		// });

		Ok(())
	}

	/// https://wayland.app/protocols/xdg-shell#xdg_surface:request:set_window_geometry
	async fn set_window_geometry(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_x: i32,
		_y: i32,
		_width: i32,
		_height: i32,
	) -> Result<()> {
		// we're gonna delegate literally all the window management
		// to 3D stuff sooo we don't care, maximized is the floating state
		Ok(())
	}

	/// https://wayland.app/protocols/xdg-shell#xdg_surface:request:ack_configure
	async fn ack_configure(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_serial: u32,
	) -> Result<()> {
		self.configured
			.store(true, std::sync::atomic::Ordering::SeqCst);
		Ok(())
	}
}

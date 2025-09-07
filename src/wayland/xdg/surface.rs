use super::{popup::Popup, positioner::Positioner, toplevel::MappedInner};
use crate::wayland::util::ClientExt;
use crate::wayland::{core::surface::SurfaceRole, display::Display, xdg::toplevel::Toplevel};
use std::sync::Arc;
use waynest::server;
pub use waynest::server::protocol::stable::xdg_shell::xdg_surface::*;
use waynest::{
	server::{Client, Dispatcher, Result},
	wire::ObjectId,
};

#[derive(Debug, Dispatcher)]
pub struct Surface {
	id: ObjectId,
	version: u32,
	pub wl_surface: Arc<crate::wayland::core::surface::Surface>,
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
			wl_surface,
			configured: Arc::new(std::sync::atomic::AtomicBool::new(false)),
		}
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
		let toplevel = client.insert(
			toplevel_id,
			Toplevel::new(
				toplevel_id,
				self.wl_surface.clone(),
				client.get::<Self>(sender_id).unwrap(),
			),
		);

		// Check if the surface already has an XDG role
		match self.wl_surface.role.get() {
			Some(SurfaceRole::XdgToplevel) => (),
			None => {
				let _ = self.wl_surface.role.set(SurfaceRole::XdgToplevel);
			}
			_ => {
				return client
					.protocol_error(
						sender_id,
						toplevel_id,
						1, // XDG_WM_BASE_ERROR_ROLE
						"Surface has a non-XDG role".to_string(),
					)
					.await;
			}
		}

		toplevel.reconfigure(client).await?;

		let toplevel_weak = Arc::downgrade(&toplevel);
		let pid = client.get::<Display>(ObjectId::DISPLAY).unwrap().pid;
		let configured = self.configured.clone();
		self.wl_surface.add_commit_handler(move |_surface, state| {
			let Some(toplevel) = toplevel_weak.upgrade() else {
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
		match self.wl_surface.role.get() {
			Some(SurfaceRole::XdgPopup) => (),
			None => {
				let _ = self.wl_surface.role.set(SurfaceRole::XdgPopup);
			}
			_ => {
				return client
					.protocol_error(
						sender_id,
						popup_id,
						1, // XDG_WM_BASE_ERROR_ROLE
						"Surface has an incomparible role".to_string(),
					)
					.await;
			}
		}

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
		let Some(parent) = client.get::<Surface>(parent) else {
			return client
				.protocol_error(
					sender_id,
					popup_id,
					3, // INVALID_POPUP_PARENT
					"Parent surface does not exist".to_string(),
				)
				.await;
		};
		let Some(panel_item) = parent.wl_surface.panel_item.lock().upgrade() else {
			return Err(server::Error::Custom(
				"Parent surface does not have a panel item".to_string(),
			));
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
				surface,
				&positioner,
			),
		);

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

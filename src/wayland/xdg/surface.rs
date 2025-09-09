use super::{popup::Popup, positioner::Positioner, toplevel::MappedInner};
use crate::nodes::items::panel::{ChildInfo, SurfaceId};
use crate::wayland::util::ClientExt;
use crate::wayland::{core::surface::SurfaceRole, display::Display, xdg::toplevel::Toplevel};
use std::sync::Arc;
use waynest::server::protocol::stable::xdg_shell::xdg_popup::XdgPopup;
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
		self.wl_surface.add_commit_handler(move |surface, state| {
			let Some(toplevel) = toplevel_weak.upgrade() else {
				return true;
			};

			let mut mapped_lock = toplevel.mapped.lock();
			if mapped_lock.is_none()
				&& configured.load(std::sync::atomic::Ordering::SeqCst)
				&& state.has_valid_buffer()
			{
				let mapped_inner = MappedInner::create(toplevel.clone(), pid);
				*surface.panel_item.lock() = Arc::downgrade(&mapped_inner.panel_item);
				mapped_lock.replace(mapped_inner);
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
		*self.wl_surface.panel_item.lock() = parent.wl_surface.panel_item.lock().clone();
		let positioner = client.get::<Positioner>(positioner).unwrap();

		let surface = client.get::<Surface>(self.id).unwrap();

		let popup = client.insert(
			popup_id,
			Popup::new(popup_id, self.version, parent.clone(), surface, &positioner),
		);

		popup.configure(client, popup_id, 0, 0, 0, 0).await?;
		let serial = client.next_event_serial();
		self.configure(client, sender_id, serial).await?;

		let Some(SurfaceId::Child(id)) = self.wl_surface.surface_id.get() else {
			return Ok(());
		};
		let Some(parent_id) = parent.wl_surface.surface_id.get() else {
			return Ok(());
		};

		let child_info = ChildInfo {
			id: *id,
			parent: parent_id.clone(),
			geometry: positioner.data().infinite_geometry(),
			z_order: 1,
			receives_input: true,
		};

		let popup_weak = Arc::downgrade(&popup);
		let configured = self.configured.clone();
		self.wl_surface.add_commit_handler(move |surface, state| {
			let Some(popup) = popup_weak.upgrade() else {
				return true;
			};
			let Some(panel_item) = surface.panel_item.lock().upgrade() else {
				return true;
			};

			if configured.load(std::sync::atomic::Ordering::SeqCst) && state.has_valid_buffer() {
				panel_item
					.backend
					.add_child(&popup.surface.wl_surface, child_info.clone());
				return false;
			}
			true
		});

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

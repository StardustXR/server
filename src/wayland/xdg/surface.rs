use super::{popup::Popup, positioner::Positioner, toplevel::MappedInner};
use crate::nodes::items::panel::{ChildInfo, SurfaceId};
use crate::wayland::{Client, WaylandError};
use crate::wayland::{
	Message, WaylandResult, core::surface::SurfaceRole, display::Display, util::ClientExt,
	xdg::toplevel::Toplevel,
};
use std::sync::Arc;
use waynest::ObjectId;
use waynest_protocols::server::stable::xdg_shell::xdg_popup::XdgPopup;
pub use waynest_protocols::server::stable::xdg_shell::xdg_surface::*;

#[derive(Debug, waynest_server::RequestDispatcher)]
#[waynest(error = crate::wayland::WaylandError)]
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

	pub async fn reconfigure(&self, client: &mut Client) -> WaylandResult<()> {
		let serial = client.next_event_serial();
		self.configure(client, self.id, serial).await
	}
}

impl XdgSurface for Surface {
	type Connection = crate::wayland::Client;

	/// https://wayland.app/protocols/xdg-shell#xdg_surface:request:destroy
	async fn destroy(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
	) -> WaylandResult<()> {
		Ok(())
	}

	/// https://wayland.app/protocols/xdg-shell#xdg_surface:request:get_toplevel
	async fn get_toplevel(
		&self,
		client: &mut Self::Connection,
		sender_id: ObjectId,
		toplevel_id: ObjectId,
	) -> WaylandResult<()> {
		let toplevel = client.insert(
			toplevel_id,
			Toplevel::new(
				toplevel_id,
				self.wl_surface.clone(),
				client.get::<Self>(sender_id).unwrap(),
			),
		);

		self.wl_surface
			.try_set_role(SurfaceRole::XdgToplevel, Error::AlreadyConstructed)
			.await?;

		let toplevel_weak = Arc::downgrade(&toplevel);
		let display = client.get::<Display>(ObjectId::DISPLAY).unwrap();
		let seat = Arc::downgrade(display.seat.get().unwrap());
		let pid = display.pid;
		let configured = self.configured.clone();
		let mut first_commit = true;
		let message_tx = client.message_sink().clone();
		self.wl_surface.add_commit_handler(move |surface, state| {
			let Some(toplevel) = toplevel_weak.upgrade() else {
				return true;
			};

			if first_commit {
				let _ = message_tx.send(Message::ReconfigureToplevel(toplevel.clone()));
				first_commit = false;
			}

			let mut mapped_lock = toplevel.mapped.lock();
			if mapped_lock.is_none()
				&& configured.load(std::sync::atomic::Ordering::SeqCst)
				&& state.has_valid_buffer()
			{
				let mapped_inner = MappedInner::create(&seat.upgrade().unwrap(), &toplevel, pid);
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
		client: &mut Self::Connection,
		sender_id: ObjectId,
		popup_id: ObjectId,
		parent: Option<ObjectId>,
		positioner: ObjectId,
	) -> WaylandResult<()> {
		self.wl_surface
			.try_set_role(SurfaceRole::XdgPopup, Error::AlreadyConstructed)
			.await?;

		let Some(parent) = parent else {
			return Err(WaylandError::Fatal {
				object_id: popup_id,
				code: 3,
				message: "Parent surface does not have an XDG role",
			});
		};
		let Some(parent) = client.get::<Surface>(parent) else {
			return Err(WaylandError::Fatal {
				object_id: popup_id,
				code: 3,
				message: "Parent surface does not exist",
			});
		};
		*self.wl_surface.panel_item.lock() = parent.wl_surface.panel_item.lock().clone();
		let positioner = client.get::<Positioner>(positioner).unwrap();

		let surface = client.get::<Surface>(self.id).unwrap();

		let popup = client.insert(popup_id, Popup::new(self.version, surface, &positioner));

		let positioner_geometry = positioner.data().infinite_geometry();

		popup
			.configure(
				client,
				popup_id,
				positioner_geometry.origin.x,
				positioner_geometry.origin.y,
				positioner_geometry.size.x as i32,
				positioner_geometry.size.y as i32,
			)
			.await?;
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
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		_x: i32,
		_y: i32,
		_width: i32,
		_height: i32,
	) -> WaylandResult<()> {
		// we're gonna delegate literally all the window management
		// to 3D stuff sooo we don't care, maximized is the floating state
		Ok(())
	}

	/// https://wayland.app/protocols/xdg-shell#xdg_surface:request:ack_configure
	async fn ack_configure(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		_serial: u32,
	) -> WaylandResult<()> {
		self.configured
			.store(true, std::sync::atomic::Ordering::SeqCst);
		Ok(())
	}
}

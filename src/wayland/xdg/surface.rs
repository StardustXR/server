use super::{popup::Popup, positioner::Positioner, toplevel::Mapped};
use crate::wayland::{core::surface::SurfaceRole, display::Display, xdg::toplevel::Toplevel};
use std::sync::Arc;
use std::sync::Weak;
pub use waynest::server::protocol::stable::xdg_shell::xdg_surface::*;
use waynest::{
	server::{Client, Dispatcher, Result},
	wire::ObjectId,
};

#[derive(Debug, Dispatcher)]
pub struct Surface {
	id: ObjectId,
	wl_surface: Weak<crate::wayland::core::surface::Surface>,
	configured: Arc<std::sync::atomic::AtomicBool>,
}
impl Surface {
	pub fn new(id: ObjectId, wl_surface: Arc<crate::wayland::core::surface::Surface>) -> Self {
		Self {
			id,
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

		{
			let mut surface_role = surface.role.lock();

			// A surface must not have any existing role when assigning a new one
			// "A surface must not have more than one role, and a role must not be assigned to more than one
			// surface at a time. However, wl_surface role-specific interfaces may reassign the role, allow
			// a role to be destroyed, or allow multiple role-specific interfaces to share the same role."
			// - xdg_surface protocol doc
			if surface_role.is_some() {
				// We should send "role" error here as per xdg_wm_base.error enum
				// But we'll ignore for now
			} else {
				surface_role.replace(SurfaceRole::XdgToplevel(toplevel.clone()));
			}
		}

		toplevel.reconfigure(client).await?;

		let pid = client.get::<Display>(ObjectId::DISPLAY).unwrap().pid;
		let configured = self.configured.clone();
		surface.add_commit_handler(move |surface, state| {
			let Some(SurfaceRole::XdgToplevel(toplevel)) = &mut *surface.role.lock() else {
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
				mapped_lock.replace(Mapped::create(toplevel.clone(), pid));
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
		let parent = client.get::<Surface>(parent.unwrap()).unwrap();
		let panel_item = match parent.wl_surface().role.lock().as_ref().unwrap() {
			SurfaceRole::XdgToplevel(toplevel) => {
				let toplevel_lock = toplevel.mapped.lock();
				toplevel_lock.as_ref().unwrap()._panel_item.clone()
			}
			SurfaceRole::XDGPopup(popup) => popup.panel_item.upgrade().unwrap(),
		};
		let positioner = client.get::<Positioner>(positioner).unwrap();

		let surface = client.get::<Surface>(self.id).unwrap();

		let popup = client.insert(
			popup_id,
			Popup::new(popup_id, &parent, &panel_item, &surface, &positioner),
		);

		{
			let wl_surface = self.wl_surface();
			let mut surface_role = wl_surface.role.lock();

			if surface_role.is_some() {
				// We should send "role" error here as per xdg_wm_base.error enum
				// But we'll ignore for now
			} else {
				surface_role.replace(SurfaceRole::XDGPopup(popup.clone()));
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

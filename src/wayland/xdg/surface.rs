use crate::wayland::{
	core::{display::Display, surface::SurfaceRole},
	xdg::toplevel::Toplevel,
};
use std::sync::Arc;
pub use waynest::server::protocol::stable::xdg_shell::xdg_surface::*;
use waynest::{
	server::{Client, Dispatcher, Result},
	wire::ObjectId,
};

use super::toplevel::Mapped;

#[derive(Debug, Dispatcher)]
pub struct Surface {
	wl_surface: Arc<crate::wayland::core::surface::Surface>,
	configured: Arc<std::sync::atomic::AtomicBool>,
}
impl Surface {
	pub fn new(wl_surface: Arc<crate::wayland::core::surface::Surface>) -> Self {
		Self {
			wl_surface,
			configured: Arc::new(std::sync::atomic::AtomicBool::new(false)),
		}
	}
}

impl XdgSurface for Surface {
	async fn destroy(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		Ok(())
	}

	async fn get_toplevel(
		&self,
		client: &mut Client,
		sender_id: ObjectId,
		toplevel_id: ObjectId,
	) -> Result<()> {
		let toplevel = client.insert(
			toplevel_id,
			Toplevel::new(toplevel_id, self.wl_surface.clone()),
		);

		{
			let mut surface_role = self.wl_surface.role.lock();

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
		let serial = client.next_event_serial();
		self.configure(client, sender_id, serial).await?;

		let pid = client.get::<Display>(ObjectId::DISPLAY).unwrap().pid;
		let configured = self.configured.clone();
		self.wl_surface.add_commit_handler(move |surface, state| {
			let Some(SurfaceRole::XdgToplevel(toplevel)) = &mut *surface.role.lock() else {
				return true;
			};

			// Only proceed if configured and has valid buffer
			let has_valid_buffer = state
				.buffer
				.as_ref()
				.map_or(false, |b| b.size.x > 0 && b.size.y > 0);

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

	async fn get_popup(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_id: ObjectId,
		_parent: Option<ObjectId>,
		_positioner: ObjectId,
	) -> Result<()> {
		todo!()
	}

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

	async fn ack_configure(
		&self,
		client: &mut Client,
		sender_id: ObjectId,
		serial: u32,
	) -> Result<()> {
		self.configured
			.store(true, std::sync::atomic::Ordering::SeqCst);
		Ok(())
	}
}

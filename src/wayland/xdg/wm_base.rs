use super::positioner::Positioner;
use crate::wayland::{WaylandError, WaylandResult, util::ClientExt, xdg::surface::Surface};

use waynest::ObjectId;
pub use waynest_protocols::server::stable::xdg_shell::xdg_wm_base::*;
use waynest_server::Client as _;

#[derive(Debug, waynest_server::RequestDispatcher)]
#[waynest(error = crate::wayland::WaylandError, connection = crate::wayland::Client)]
pub struct WmBase {
	version: u32,
	id: ObjectId,
}
impl WmBase {
	pub fn new(id: ObjectId, version: u32) -> Self {
		Self { version, id }
	}
}
impl XdgWmBase for WmBase {
	type Connection = crate::wayland::Client;

	async fn destroy(
		&self,
		client: &mut Self::Connection,
		_sender_id: ObjectId,
	) -> WaylandResult<()> {
		client.remove(self.id);
		Ok(())
	}

	async fn create_positioner(
		&self,
		client: &mut Self::Connection,
		_sender_id: ObjectId,
		id: ObjectId,
	) -> WaylandResult<()> {
		client.insert(id, Positioner::new(id))?;
		Ok(())
	}

	async fn get_xdg_surface(
		&self,
		client: &mut Self::Connection,
		_sender_id: ObjectId,
		xdg_surface_id: ObjectId,
		wl_surface_id: ObjectId,
	) -> WaylandResult<()> {
		let wl_surface = client.try_get::<crate::wayland::core::surface::Surface>(wl_surface_id)?;
		match wl_surface.role.get() {
			None => (),
			Some(_) => {
				return Err(WaylandError::Fatal {
					object_id: wl_surface_id,
					code: Error::Role as u32,
					message: "Wayland surface has role",
				});
			}
		};
		let xdg_surface = Surface::new(xdg_surface_id, self.version, wl_surface);
		client.insert(xdg_surface_id, xdg_surface)?;

		Ok(())
	}

	async fn pong(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		_serial: u32,
	) -> WaylandResult<()> {
		Ok(())
	}
}

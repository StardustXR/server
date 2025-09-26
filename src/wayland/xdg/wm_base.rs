use super::positioner::Positioner;
use crate::wayland::{WaylandError, WaylandResult, util::ClientExt, xdg::surface::Surface};

use waynest::ObjectId;
pub use waynest_protocols::server::stable::xdg_shell::xdg_wm_base::*;

#[derive(Debug, waynest_server::RequestDispatcher, Default)]
#[waynest(error = crate::wayland::WaylandError)]
pub struct WmBase {
	pub version: u32,
}
impl XdgWmBase for WmBase {
	type Connection = crate::wayland::Client;

	async fn destroy(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
	) -> WaylandResult<()> {
		Ok(())
	}

	async fn create_positioner(
		&self,
		client: &mut Self::Connection,
		_sender_id: ObjectId,
		id: ObjectId,
	) -> WaylandResult<()> {
		client.insert(id, Positioner::default());
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
		client.insert(xdg_surface_id, xdg_surface);

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

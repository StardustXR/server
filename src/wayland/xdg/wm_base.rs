use super::positioner::Positioner;
use crate::wayland::{core::surface::SurfaceRole, util::ClientExt, xdg::surface::Surface};
pub use waynest::server::protocol::stable::xdg_shell::xdg_wm_base::*;
use waynest::{
	server::{Client, Dispatcher, Result},
	wire::ObjectId,
};

#[derive(Debug, Dispatcher, Default)]
pub struct WmBase {
	pub version: u32,
}
impl XdgWmBase for WmBase {
	async fn destroy(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		Ok(())
	}

	async fn create_positioner(
		&self,
		client: &mut Client,
		_sender_id: ObjectId,
		id: ObjectId,
	) -> Result<()> {
		client.insert(id, Positioner::default());
		Ok(())
	}

	async fn get_xdg_surface(
		&self,
		client: &mut Client,
		sender_id: ObjectId,
		xdg_surface_id: ObjectId,
		wl_surface_id: ObjectId,
	) -> Result<()> {
		let wl_surface = client
			.get::<crate::wayland::core::surface::Surface>(wl_surface_id)
			.ok_or(waynest::server::Error::Custom(
				"can't get wayland surface id".to_string(),
			))?;
		match wl_surface.role.get() {
			Some(SurfaceRole::XdgToplevel | SurfaceRole::XdgPopup) => (),
			None => (),
			_ => {
				client
					.protocol_error(
						sender_id,
						xdg_surface_id,
						0,
						"invalid surface role".to_string(),
					)
					.await?
			}
		};
		let xdg_surface = Surface::new(xdg_surface_id, self.version, wl_surface);
		client.insert(xdg_surface_id, xdg_surface);

		Ok(())
	}

	async fn pong(&self, _client: &mut Client, _sender_id: ObjectId, _serial: u32) -> Result<()> {
		Ok(())
	}
}

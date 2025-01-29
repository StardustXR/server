use crate::wayland::{
	util::ObjectIdExt,
	xdg::surface::{Surface, XdgSurface},
};
pub use waynest::server::protocol::stable::xdg_shell::xdg_wm_base::*;
use waynest::{
	server::{
		protocol::stable::xdg_shell::xdg_positioner::XdgPositioner, Client, Dispatcher, Object,
		Result,
	},
	wire::ObjectId,
};

use super::popup::Positioner;

#[derive(Debug, Dispatcher, Default)]
pub struct WmBase;
impl XdgWmBase for WmBase {
	async fn destroy(&self, _object: &Object, _client: &mut Client) -> Result<()> {
		Ok(())
	}

	async fn create_positioner(
		&self,
		_object: &Object,
		client: &mut Client,
		id: ObjectId,
	) -> Result<()> {
		client.insert(Positioner::default().into_object(id));
		Ok(())
	}

	async fn get_xdg_surface(
		&self,
		_object: &Object,
		client: &mut Client,
		id: ObjectId,
		surface: ObjectId,
	) -> Result<()> {
		let wl_surface = surface
			.upgrade(client)
			.ok_or(waynest::server::Error::Internal)?;
		client.insert(Surface::new(wl_surface.as_dispatcher()?).into_object(id));

		Ok(())
	}

	async fn pong(&self, _object: &Object, _client: &mut Client, _serial: u32) -> Result<()> {
		Ok(())
	}
}

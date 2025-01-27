use crate::wayland::xdg::surface::{Surface, XdgSurface};
pub use waynest::server::protocol::stable::xdg_shell::xdg_wm_base::*;
use waynest::{
	server::{Client, Dispatcher, Object, Result},
	wire::ObjectId,
};

#[derive(Debug, Dispatcher, Default)]
pub struct WmBase;

impl XdgWmBase for WmBase {
	async fn destroy(&self, _object: &Object, _client: &mut Client) -> Result<()> {
		todo!()
	}

	async fn create_positioner(
		&self,
		_object: &Object,
		_client: &mut Client,
		_id: ObjectId,
	) -> Result<()> {
		todo!()
	}

	async fn get_xdg_surface(
		&self,
		_object: &Object,
		client: &mut Client,
		id: ObjectId,
		surface: ObjectId,
	) -> Result<()> {
		client.insert(Surface::new(surface).into_object(id));

		Ok(())
	}

	async fn pong(&self, _object: &Object, _client: &mut Client, _serial: u32) -> Result<()> {
		todo!()
	}
}

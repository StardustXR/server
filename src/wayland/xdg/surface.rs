use crate::wayland::xdg::toplevel::{Toplevel, XdgToplevel};
pub use waynest::server::protocol::stable::xdg_shell::xdg_surface::*;
use waynest::{
	server::{Client, Dispatcher, Object, Result},
	wire::ObjectId,
};

#[derive(Debug, Dispatcher)]
pub struct Surface {
	wl_surface: ObjectId,
}
impl Surface {
	pub fn new(wl_surface: ObjectId) -> Self {
		Self { wl_surface }
	}
}

impl XdgSurface for Surface {
	async fn destroy(&self, _object: &Object, _client: &mut Client) -> Result<()> {
		todo!()
	}

	async fn get_toplevel(
		&self,
		_object: &Object,
		client: &mut Client,
		id: ObjectId,
	) -> Result<()> {
		client.insert(Toplevel::default().into_object(id));

		Ok(())
	}

	async fn get_popup(
		&self,
		_object: &Object,
		_client: &mut Client,
		_id: ObjectId,
		_parent: Option<ObjectId>,
		_positioner: ObjectId,
	) -> Result<()> {
		todo!()
	}

	async fn set_window_geometry(
		&self,
		_object: &Object,
		_client: &mut Client,
		_x: i32,
		_y: i32,
		_width: i32,
		_height: i32,
	) -> Result<()> {
		todo!()
	}

	async fn ack_configure(
		&self,
		_object: &Object,
		_client: &mut Client,
		_serial: u32,
	) -> Result<()> {
		todo!()
	}
}

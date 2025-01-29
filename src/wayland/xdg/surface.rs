use crate::wayland::{
	core::display::Display,
	xdg::toplevel::{Toplevel, XdgToplevel},
};
use std::sync::Arc;
pub use waynest::server::protocol::stable::xdg_shell::xdg_surface::*;
use waynest::{
	server::{self, Client, Dispatcher, Object, Result},
	wire::ObjectId,
};

#[derive(Debug, Dispatcher)]
pub struct Surface {
	wl_surface: Arc<crate::wayland::core::surface::Surface>,
}
impl Surface {
	pub fn new(wl_surface: Arc<crate::wayland::core::surface::Surface>) -> Self {
		Self { wl_surface }
	}
}

impl XdgSurface for Surface {
	async fn destroy(&self, _object: &Object, _client: &mut Client) -> Result<()> {
		Ok(())
	}

	async fn get_toplevel(
		&self,
		_object: &Object,
		client: &mut Client,
		id: ObjectId,
	) -> Result<()> {
		let pid = client
			.get_object(&ObjectId::DISPLAY)
			.unwrap()
			.as_dispatcher::<Display>()
			.unwrap()
			.pid
			.clone();

		let size = self.wl_surface.size().ok_or(server::Error::Internal)?;
		client.insert(Toplevel::new(pid, size).into_object(id));

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
		// we're gonna delegate literally all the window management
		// to 3D stuff sooo we don't care, maximized is the floating state
		Ok(())
	}

	async fn ack_configure(
		&self,
		_object: &Object,
		_client: &mut Client,
		_serial: u32,
	) -> Result<()> {
		// just gonna apply state immediately, it's fiiiine
		Ok(())
	}
}

use crate::wayland::{Client, WaylandResult};
use waynest::ObjectId;
use waynest_protocols::server::unstable::xdg_decoration_unstable_v1::{
	zxdg_decoration_manager_v1::*, zxdg_toplevel_decoration_v1::*,
};
use waynest_server::Client as _;

#[derive(Debug, waynest_server::RequestDispatcher)]
#[waynest(error = crate::wayland::WaylandError, connection = crate::wayland::Client)]
pub struct XdgDecorationManager {
	pub _version: u32,
	pub id: ObjectId,
}
impl ZxdgDecorationManagerV1 for XdgDecorationManager {
	type Connection = Client;

	async fn destroy(
		&self,
		client: &mut Self::Connection,
		_sender_id: ObjectId,
	) -> WaylandResult<()> {
		client.remove(self.id);
		Ok(())
	}

	async fn get_toplevel_decoration(
		&self,
		client: &mut Self::Connection,
		_sender_id: ObjectId,
		id: ObjectId,
		_toplevel: ObjectId,
	) -> WaylandResult<()> {
		client.insert(id, XdgDecoration { id })?;
		Ok(())
	}
}

#[derive(Debug, waynest_server::RequestDispatcher)]
#[waynest(error = crate::wayland::WaylandError, connection = crate::wayland::Client)]
pub struct XdgDecoration {
	id: ObjectId,
}
impl ZxdgToplevelDecorationV1 for XdgDecoration {
	type Connection = Client;

	async fn destroy(
		&self,
		client: &mut Self::Connection,
		sender_id: ObjectId,
	) -> WaylandResult<()> {
		client.remove(sender_id);
		Ok(())
	}

	async fn set_mode(
		&self,
		client: &mut Self::Connection,
		_sender_id: ObjectId,
		_mode: Mode,
	) -> WaylandResult<()> {
		// TODO: proper robust implementation where configure must be sent before first buffer attach
		self.configure(client, self.id, Mode::ServerSide).await
	}

	async fn unset_mode(
		&self,
		client: &mut Self::Connection,
		_sender_id: ObjectId,
	) -> WaylandResult<()> {
		// TODO: proper robust implementation where configure must be sent before first buffer attach
		self.configure(client, self.id, Mode::ServerSide).await
	}
}

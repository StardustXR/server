use crate::wayland::WaylandResult;
use waynest::Fixed;
use waynest::ObjectId;
pub use waynest_protocols::server::stable::viewporter::wp_viewport::*;
pub use waynest_protocols::server::stable::viewporter::wp_viewporter::*;
use waynest_server::Client as _;

// This is a barebones/stub no-op implementation of wp_viewporter to make xwayland apps work

#[derive(Debug, waynest_server::RequestDispatcher)]
#[waynest(error = crate::wayland::WaylandError, connection = crate::wayland::Client)]
pub struct Viewporter {
	id: ObjectId,
}

impl Viewporter {
	pub fn new(id: ObjectId) -> Self {
		Self { id }
	}
}

impl WpViewporter for Viewporter {
	type Connection = crate::wayland::Client;

	async fn destroy(
		&self,
		client: &mut Self::Connection,
		_sender_id: ObjectId,
	) -> WaylandResult<()> {
		client.remove(self.id);
		Ok(())
	}

	async fn get_viewport(
		&self,
		client: &mut Self::Connection,
		_sender_id: ObjectId,
		id: ObjectId,
		surface_id: ObjectId,
	) -> WaylandResult<()> {
		let viewport = Viewport::new(id, surface_id);
		client.insert(id, viewport)?;
		Ok(())
	}
}

#[derive(Debug, waynest_server::RequestDispatcher)]
#[waynest(error = crate::wayland::WaylandError, connection = crate::wayland::Client)]
pub struct Viewport {
	id: ObjectId,
	_surface_id: ObjectId,
}

impl Viewport {
	pub fn new(id: ObjectId, surface_id: ObjectId) -> Self {
		Self {
			id,
			_surface_id: surface_id,
		}
	}
}

impl WpViewport for Viewport {
	type Connection = crate::wayland::Client;

	async fn destroy(
		&self,
		client: &mut Self::Connection,
		_sender_id: ObjectId,
	) -> WaylandResult<()> {
		client.remove(self.id);
		Ok(())
	}

	async fn set_source(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		_x: Fixed,
		_y: Fixed,
		_width: Fixed,
		_height: Fixed,
	) -> WaylandResult<()> {
		Ok(())
	}

	async fn set_destination(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		_width: i32,
		_height: i32,
	) -> WaylandResult<()> {
		Ok(())
	}
}

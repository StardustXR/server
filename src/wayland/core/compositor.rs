use super::surface::WL_SURFACE_REGISTRY;
use crate::wayland::{WaylandError, WaylandResult};
use crate::wayland::{core::surface::Surface, util::ClientExt};
use waynest::ObjectId;
use waynest_protocols::server::core::wayland::wl_surface::WlSurface;
pub use waynest_protocols::server::core::wayland::{wl_compositor::*, wl_region::*};
use waynest_server::RequestDispatcher;

#[derive(Debug, waynest_server::RequestDispatcher, Default)]
#[waynest(error = WaylandError)]
pub struct Compositor;
impl WlCompositor for Compositor {
	type Connection = crate::wayland::Client;

	/// https://wayland.app/protocols/wayland#wl_compositor:request:create_surface
	async fn create_surface(
		&self,
		client: &mut Self::Connection,
		_sender_id: ObjectId,
		id: ObjectId,
	) -> WaylandResult<()> {
		let surface = client.insert(id, Surface::new(client, id));
		if let Some(output) = client.display().output.get() {
			surface.enter(client, id, output.id).await?;
		}
		WL_SURFACE_REGISTRY.add_raw(&surface);

		Ok(())
	}

	/// https://wayland.app/protocols/wayland#wl_compositor:request:create_region
	async fn create_region(
		&self,
		client: &mut Self::Connection,
		_sender_id: ObjectId,
		id: ObjectId,
	) -> WaylandResult<()> {
		client.insert(id, Region { id });
		Ok(())
	}
}

#[derive(Debug, RequestDispatcher)]
#[waynest(error = WaylandError)]
pub struct Region {
	id: ObjectId,
}
impl WlRegion for Region {
	type Connection = crate::wayland::Client;

	/// https://wayland.app/protocols/wayland#wl_region:request:add
	async fn add(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		_x: i32,
		_y: i32,
		_width: i32,
		_height: i32,
	) -> WaylandResult<()> {
		Ok(())
	}

	/// https://wayland.app/protocols/wayland#wl_region:request:subtract
	async fn subtract(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		_x: i32,
		_y: i32,
		_width: i32,
		_height: i32,
	) -> WaylandResult<()> {
		Ok(())
	}

	/// https://wayland.app/protocols/wayland#wl_region:request:destroy
	async fn destroy(
		&self,
		client: &mut Self::Connection,
		_sender_id: ObjectId,
	) -> WaylandResult<()> {
		client.remove(self.id);
		Ok(())
	}
}

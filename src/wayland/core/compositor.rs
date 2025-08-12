use super::surface::WL_SURFACE_REGISTRY;
use crate::wayland::{core::surface::Surface, util::ClientExt};
pub use waynest::server::protocol::core::wayland::wl_compositor::*;
use waynest::{
	server::{
		Client, Dispatcher, Result,
		protocol::core::wayland::{wl_region::WlRegion, wl_surface::WlSurface},
	},
	wire::ObjectId,
};

#[derive(Debug, Dispatcher, Default)]
pub struct Compositor;
impl WlCompositor for Compositor {
	/// https://wayland.app/protocols/wayland#wl_compositor:request:create_surface
	async fn create_surface(
		&self,
		client: &mut Client,
		_sender_id: ObjectId,
		id: ObjectId,
	) -> Result<()> {
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
		client: &mut Client,
		_sender_id: ObjectId,
		id: ObjectId,
	) -> Result<()> {
		client.insert(id, Region::default());
		Ok(())
	}
}

#[derive(Debug, Dispatcher, Default)]
pub struct Region {}
impl WlRegion for Region {
	/// https://wayland.app/protocols/wayland#wl_region:request:add
	async fn add(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_x: i32,
		_y: i32,
		_width: i32,
		_height: i32,
	) -> Result<()> {
		Ok(())
	}

	/// https://wayland.app/protocols/wayland#wl_region:request:subtract
	async fn subtract(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_x: i32,
		_y: i32,
		_width: i32,
		_height: i32,
	) -> Result<()> {
		Ok(())
	}

	/// https://wayland.app/protocols/wayland#wl_region:request:destroy
	async fn destroy(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		Ok(())
	}
}

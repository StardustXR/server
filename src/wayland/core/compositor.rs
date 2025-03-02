use super::surface::WL_SURFACE_REGISTRY;
use crate::wayland::core::surface::Surface;
pub use waynest::server::protocol::core::wayland::wl_compositor::*;
use waynest::{
	server::{Client, Dispatcher, Result, protocol::core::wayland::wl_region::WlRegion},
	wire::ObjectId,
};

#[derive(Debug, Dispatcher, Default)]
pub struct Compositor;
impl WlCompositor for Compositor {
	async fn create_surface(
		&self,
		client: &mut Client,
		_sender_id: ObjectId,
		id: ObjectId,
	) -> Result<()> {
		let surface = client.insert(id, Surface::new(client, id));
		WL_SURFACE_REGISTRY.add_raw(&surface);

		Ok(())
	}

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

	async fn destroy(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		Ok(())
	}
}

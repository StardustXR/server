use super::surface::WL_SURFACE_REGISTRY;
use crate::wayland::core::surface::Surface;
pub use waynest::server::protocol::core::wayland::wl_compositor::*;
use waynest::{
	server::{
		protocol::core::wayland::{wl_region::WlRegion, wl_surface::WlSurface},
		Client, Dispatcher, Object, Result,
	},
	wire::ObjectId,
};

#[derive(Debug, Dispatcher, Default)]
pub struct Compositor;
impl WlCompositor for Compositor {
	async fn create_surface(
		&self,
		_object: &Object,
		client: &mut Client,
		id: ObjectId,
	) -> Result<()> {
		let surface = Surface::new(client).into_object(id);
		WL_SURFACE_REGISTRY.add_raw(&surface.as_dispatcher()?);
		client.insert(surface);

		Ok(())
	}

	async fn create_region(
		&self,
		_object: &Object,
		client: &mut Client,
		id: ObjectId,
	) -> Result<()> {
		client.insert(Region::default().into_object(id));
		Ok(())
	}
}

#[derive(Debug, Dispatcher, Default)]
pub struct Region {}
impl WlRegion for Region {
	async fn add(
		&self,
		_object: &Object,
		_client: &mut Client,
		_x: i32,
		_y: i32,
		_width: i32,
		_height: i32,
	) -> Result<()> {
		Ok(())
	}

	async fn subtract(
		&self,
		_object: &Object,
		_client: &mut Client,
		_x: i32,
		_y: i32,
		_width: i32,
		_height: i32,
	) -> Result<()> {
		Ok(())
	}

	async fn destroy(&self, _object: &Object, _client: &mut Client) -> Result<()> {
		Ok(())
	}
}

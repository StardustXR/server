use waynest::{
	server::{protocol::core::wayland::wl_output::Transform, Client, Dispatcher, Object, Result},
	wire::ObjectId,
};

pub use waynest::server::protocol::core::wayland::wl_surface::*;

#[derive(Debug, Default)]
struct State {}

#[derive(Debug, Default)]
struct DoubleBuffer {
	current: State,
	pending: State,
}

#[derive(Debug, Dispatcher, Default)]
pub struct Surface {
	state: DoubleBuffer,
}

impl WlSurface for Surface {
	async fn attach(
		&self,
		_object: &Object,
		_client: &mut Client,
		_buffer: Option<ObjectId>,
		_x: i32,
		_y: i32,
	) -> Result<()> {
		todo!()
	}

	async fn damage(
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

	async fn frame(
		&self,
		_object: &Object,
		_client: &mut Client,
		_callback: ObjectId,
	) -> Result<()> {
		todo!()
	}

	async fn set_opaque_region(
		&self,
		_object: &Object,
		_client: &mut Client,
		_region: Option<ObjectId>,
	) -> Result<()> {
		todo!()
	}

	async fn set_input_region(
		&self,
		_object: &Object,
		_client: &mut Client,
		_region: Option<ObjectId>,
	) -> Result<()> {
		todo!()
	}

	async fn commit(&self, _object: &Object, _client: &mut Client) -> Result<()> {
		// FIXME: commit state

		Ok(())
	}

	async fn set_buffer_transform(
		&self,
		_object: &Object,
		_client: &mut Client,
		_transform: Transform,
	) -> Result<()> {
		todo!()
	}

	async fn set_buffer_scale(
		&self,
		_object: &Object,
		_client: &mut Client,
		_scale: i32,
	) -> Result<()> {
		todo!()
	}

	async fn damage_buffer(
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

	async fn offset(&self, _object: &Object, _client: &mut Client, _x: i32, _y: i32) -> Result<()> {
		todo!()
	}

	async fn destroy(&self, _object: &Object, _client: &mut Client) -> Result<()> {
		Ok(())
	}
}

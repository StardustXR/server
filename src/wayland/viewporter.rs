use waynest::{
	server::{Client, Dispatcher, Result},
	wire::{Fixed, ObjectId},
};

pub use waynest::server::protocol::stable::viewporter::wp_viewport::*;
pub use waynest::server::protocol::stable::viewporter::wp_viewporter::*;

// This is a barebones/stub no-op implementation of wp_viewporter to make xwayland apps work

#[derive(Debug, Dispatcher, Default)]
pub struct Viewporter;

impl WpViewporter for Viewporter {
	async fn destroy(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		Ok(())
	}

	async fn get_viewport(
		&self,
		client: &mut Client,
		_sender_id: ObjectId,
		id: ObjectId,
		surface_id: ObjectId,
	) -> Result<()> {
		let viewport = Viewport::new(id, surface_id);
		client.insert(id, viewport);
		Ok(())
	}
}

#[derive(Debug, Dispatcher)]
pub struct Viewport {
	id: ObjectId,
	surface_id: ObjectId,
}

impl Viewport {
	pub fn new(id: ObjectId, surface_id: ObjectId) -> Self {
		Self { id, surface_id }
	}
}

impl WpViewport for Viewport {
	async fn destroy(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		Ok(())
	}

	async fn set_source(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_x: Fixed,
		_y: Fixed,
		_width: Fixed,
		_height: Fixed,
	) -> Result<()> {
		Ok(())
	}

	async fn set_destination(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_width: i32,
		_height: i32,
	) -> Result<()> {
		Ok(())
	}
}

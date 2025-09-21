use super::{
	positioner::{Positioner, PositionerData},
	surface::Surface,
};
use crate::nodes::items::panel::SurfaceId;
use parking_lot::Mutex;
use rand::Rng;
use std::sync::Arc;
use waynest::{
	server::{Client, Dispatcher, Result, protocol::stable::xdg_shell::xdg_popup::XdgPopup},
	wire::ObjectId,
};

#[derive(Debug, Dispatcher)]
pub struct Popup {
	version: u32,
	pub surface: Arc<Surface>,
	positioner_data: Mutex<PositionerData>,
}
impl Popup {
	pub fn new(version: u32, surface: Arc<Surface>, positioner: &Positioner) -> Self {
		let _ = surface
			.wl_surface
			.surface_id
			.set(SurfaceId::Child(rand::rng().random()));

		let positioner_data = positioner.data();
		Self {
			version,
			surface,
			positioner_data: Mutex::new(positioner_data),
		}
	}
}
impl XdgPopup for Popup {
	/// https://wayland.app/protocols/xdg-shell#xdg_popup:request:grab
	async fn grab(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_seat: ObjectId,
		_serial: u32,
	) -> Result<()> {
		Ok(())
	}

	/// https://wayland.app/protocols/xdg-shell#xdg_popup:request:reposition
	async fn reposition(
		&self,
		client: &mut Client,
		sender_id: ObjectId,
		positioner: ObjectId,
		token: u32,
	) -> Result<()> {
		let positioner = client.get::<Positioner>(positioner).unwrap();
		let positioner_data = positioner.data();
		*self.positioner_data.lock() = positioner_data;
		if self.version >= 5 {
			self.repositioned(client, sender_id, token).await?;
		}
		let geometry = positioner_data.infinite_geometry();
		self.configure(
			client,
			sender_id,
			geometry.origin.x,
			geometry.origin.y,
			geometry.size.x as i32,
			geometry.size.y as i32,
		)
		.await?;
		self.surface.reconfigure(client).await?;

		let Some(panel_item) = self.surface.wl_surface.panel_item.lock().upgrade() else {
			return Ok(());
		};
		panel_item
			.backend
			.reposition_child(&self.surface.wl_surface, geometry);
		Ok(())
	}

	/// https://wayland.app/protocols/xdg-shell#xdg_popup:request:destroy
	async fn destroy(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		Ok(())
	}
}
impl Drop for Popup {
	fn drop(&mut self) {
		let Some(panel_item) = self.surface.wl_surface.panel_item.lock().upgrade() else {
			return;
		};
		panel_item.backend.remove_child(&self.surface.wl_surface);
	}
}

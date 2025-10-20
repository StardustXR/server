use super::{
	positioner::{Positioner, PositionerData},
	surface::Surface,
};
use crate::nodes::items::panel::SurfaceId;
use crate::wayland::WaylandResult;
use parking_lot::Mutex;
use rand::Rng;
use std::sync::Arc;
use waynest::ObjectId;
use waynest_protocols::server::stable::xdg_shell::xdg_popup::XdgPopup;
use waynest_server::Client as _;

#[derive(Debug, waynest_server::RequestDispatcher)]
#[waynest(error = crate::wayland::WaylandError, connection = crate::wayland::Client)]
pub struct Popup {
	version: u32,
	pub surface: Arc<Surface>,
	positioner_data: Mutex<PositionerData>,
	id: ObjectId,
}
impl Popup {
	pub fn new(version: u32, surface: Arc<Surface>, positioner: &Positioner, id: ObjectId) -> Self {
		let _ = surface
			.wl_surface
			.surface_id
			.set(SurfaceId::Child(rand::rng().random()));

		let positioner_data = positioner.data();
		Self {
			version,
			surface,
			positioner_data: Mutex::new(positioner_data),
			id,
		}
	}
}
impl XdgPopup for Popup {
	type Connection = crate::wayland::Client;

	/// https://wayland.app/protocols/xdg-shell#xdg_popup:request:grab
	async fn grab(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		_seat: ObjectId,
		_serial: u32,
	) -> WaylandResult<()> {
		Ok(())
	}

	/// https://wayland.app/protocols/xdg-shell#xdg_popup:request:reposition
	async fn reposition(
		&self,
		client: &mut Self::Connection,
		sender_id: ObjectId,
		positioner: ObjectId,
		token: u32,
	) -> WaylandResult<()> {
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
	async fn destroy(
		&self,
		client: &mut Self::Connection,
		_sender_id: ObjectId,
	) -> WaylandResult<()> {
		client.remove(self.id);
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

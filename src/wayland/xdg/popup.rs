use super::{
	backend::XdgBackend,
	positioner::{Positioner, PositionerData},
	surface::Surface,
};
use crate::{
	nodes::items::panel::{Geometry, PanelItem, SurfaceId},
	wayland::util::DoubleBuffer,
};
use parking_lot::Mutex;
use rand::Rng;
use std::sync::{Arc, Weak, atomic::AtomicBool};
use waynest::{
	server::{Client, Dispatcher, Result, protocol::stable::xdg_shell::xdg_popup::XdgPopup},
	wire::ObjectId,
};

#[derive(Debug, Dispatcher)]
pub struct Popup {
	id: ObjectId,
	version: u32,
	parent: Arc<Surface>,
	surface: Arc<Surface>,
	pub panel_item: Weak<PanelItem<XdgBackend>>,
	positioner_data: Mutex<PositionerData>,
	geometry: DoubleBuffer<Geometry>,
	mapped: AtomicBool,
}
impl Popup {
	pub fn new(
		id: ObjectId,
		version: u32,
		parent: Arc<Surface>,
		panel_item: &Arc<PanelItem<XdgBackend>>,
		surface: Arc<Surface>,
		positioner: &Positioner,
	) -> Self {
		let _ = surface
			.wl_surface
			.surface_id
			.set(SurfaceId::Child(rand::thread_rng().gen_range(0..u64::MAX)));

		let positioner_data = positioner.data();
		Self {
			id,
			version,
			parent,
			surface,
			panel_item: Arc::downgrade(panel_item),
			positioner_data: Mutex::new(positioner_data),
			geometry: DoubleBuffer::new(positioner_data.infinite_geometry()),
			mapped: AtomicBool::new(false),
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
		Ok(())
	}

	/// https://wayland.app/protocols/xdg-shell#xdg_popup:request:destroy
	async fn destroy(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		Ok(())
	}
}

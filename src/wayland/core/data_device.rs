use crate::wayland::{Client, WaylandResult};
use std::os::fd::OwnedFd;
use waynest::ObjectId;
use waynest_protocols::server::core::wayland::{
	wl_data_device::*, wl_data_device_manager::*, wl_data_offer::WlDataOffer, wl_data_source::*,
};

// TODO: actually implement this

#[derive(Debug, waynest_server::RequestDispatcher)]
#[waynest(error = crate::wayland::WaylandError)]
pub struct DataDeviceManager;
impl WlDataDeviceManager for DataDeviceManager {
	type Connection = Client;

	async fn create_data_source(
		&self,
		client: &mut Self::Connection,
		_sender_id: ObjectId,
		id: ObjectId,
	) -> WaylandResult<()> {
		client.insert(id, DataSource);
		Ok(())
	}

	async fn get_data_device(
		&self,
		client: &mut Client,
		_sender_id: ObjectId,
		id: ObjectId,
		_seat: ObjectId,
	) -> WaylandResult<()> {
		client.insert(id, DataDevice);
		Ok(())
	}
}

#[derive(Debug, waynest_server::RequestDispatcher)]
#[waynest(error = crate::wayland::WaylandError)]
pub struct DataSource;
impl WlDataSource for DataSource {
	type Connection = Client;

	async fn send(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		_mime_type: String,
		_fd: OwnedFd,
	) -> WaylandResult<()> {
		Ok(())
	}

	async fn offer(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		_mime_type: String,
	) -> WaylandResult<()> {
		Ok(())
	}

	async fn destroy(&self, _client: &mut Self::Connection, _sender_id: ObjectId) -> WaylandResult<()> {
		Ok(())
	}

	async fn set_actions(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		_dnd_actions: DndAction,
	) -> WaylandResult<()> {
		Ok(())
	}
}

#[derive(Debug, waynest_server::RequestDispatcher)]
#[waynest(error = crate::wayland::WaylandError)]
pub struct DataDevice;
impl WlDataDevice for DataDevice {
	type Connection = Client;

	async fn start_drag(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		_source: Option<ObjectId>,
		_origin: ObjectId,
		_icon: Option<ObjectId>,
		_serial: u32,
	) -> WaylandResult<()> {
		Ok(())
	}

	async fn set_selection(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		_source: Option<ObjectId>,
		_serial: u32,
	) -> WaylandResult<()> {
		Ok(())
	}

	async fn release(&self, _client: &mut Self::Connection, _sender_id: ObjectId) -> WaylandResult<()> {
		Ok(())
	}
}

#[derive(Debug, waynest_server::RequestDispatcher)]
#[waynest(error = crate::wayland::WaylandError)]
pub struct DataOffer;
impl WlDataOffer for DataOffer {
	type Connection = Client;

	async fn accept(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		_serial: u32,
		_mime_type: Option<String>,
	) -> WaylandResult<()> {
		Ok(())
	}

	async fn receive(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		_mime_type: String,
		_fd: OwnedFd,
	) -> WaylandResult<()> {
		Ok(())
	}

	async fn destroy(&self, _client: &mut Self::Connection, _sender_id: ObjectId) -> WaylandResult<()> {
		Ok(())
	}

	async fn finish(&self, _client: &mut Self::Connection, _sender_id: ObjectId) -> WaylandResult<()> {
		Ok(())
	}

	async fn set_actions(
		&self,
		_client: &mut Self::Connection,
		_sender_id: ObjectId,
		_dnd_actions: DndAction,
		_preferred_action: DndAction,
	) -> WaylandResult<()> {
		Ok(())
	}
}

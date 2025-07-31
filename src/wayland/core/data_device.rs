use std::os::fd::OwnedFd;
use waynest::{
	server::{
		Client, Dispatcher, Result,
		protocol::core::wayland::{
			wl_data_device::*, wl_data_device_manager::*, wl_data_offer::WlDataOffer,
			wl_data_source::*,
		},
	},
	wire::ObjectId,
};

// TODO: actually implement this

#[derive(Debug, Dispatcher)]
pub struct DataDeviceManager;
impl WlDataDeviceManager for DataDeviceManager {
	async fn create_data_source(
		&self,
		client: &mut Client,
		_sender_id: ObjectId,
		id: ObjectId,
	) -> Result<()> {
		client.insert(id, DataSource);
		Ok(())
	}

	async fn get_data_device(
		&self,
		client: &mut Client,
		_sender_id: ObjectId,
		id: ObjectId,
		_seat: ObjectId,
	) -> Result<()> {
		client.insert(id, DataDevice);
		Ok(())
	}
}

#[derive(Debug, Dispatcher)]
pub struct DataSource;
impl WlDataSource for DataSource {
	async fn send(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_mime_type: String,
		_fd: OwnedFd,
	) -> Result<()> {
		Ok(())
	}

	async fn offer(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_mime_type: String,
	) -> Result<()> {
		Ok(())
	}

	async fn destroy(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		Ok(())
	}

	async fn set_actions(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_dnd_actions: DndAction,
	) -> Result<()> {
		Ok(())
	}
}

#[derive(Debug, Dispatcher)]
pub struct DataDevice;
impl WlDataDevice for DataDevice {
	async fn start_drag(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_source: Option<ObjectId>,
		_origin: ObjectId,
		_icon: Option<ObjectId>,
		_serial: u32,
	) -> Result<()> {
		Ok(())
	}

	async fn set_selection(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_source: Option<ObjectId>,
		_serial: u32,
	) -> Result<()> {
		Ok(())
	}

	async fn release(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		Ok(())
	}
}

#[derive(Debug, Dispatcher)]
pub struct DataOffer;
impl WlDataOffer for DataOffer {
	async fn accept(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_serial: u32,
		_mime_type: Option<String>,
	) -> Result<()> {
		Ok(())
	}

	async fn receive(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_mime_type: String,
		_fd: OwnedFd,
	) -> Result<()> {
		Ok(())
	}

	async fn destroy(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		Ok(())
	}

	async fn finish(&self, _client: &mut Client, _sender_id: ObjectId) -> Result<()> {
		Ok(())
	}

	async fn set_actions(
		&self,
		_client: &mut Client,
		_sender_id: ObjectId,
		_dnd_actions: DndAction,
		_preferred_action: DndAction,
	) -> Result<()> {
		Ok(())
	}
}

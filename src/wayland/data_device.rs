use smithay::reexports::wayland_server::{
	Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
	protocol::{
		wl_data_device::{
			Request::{Release, SetSelection, StartDrag},
			WlDataDevice,
		},
		wl_data_device_manager::{
			Request::{CreateDataSource, GetDataDevice},
			WlDataDeviceManager,
		},
		wl_data_source::{
			Request::{Destroy, Offer, SetActions},
			WlDataSource,
		},
	},
};

use super::state::WaylandState;

impl GlobalDispatch<WlDataDeviceManager, (), WaylandState> for WaylandState {
	fn bind(
		_state: &mut WaylandState,
		_handle: &DisplayHandle,
		_client: &Client,
		resource: New<WlDataDeviceManager>,
		_global_data: &(),
		data_init: &mut DataInit<'_, WaylandState>,
	) {
		let _resource = data_init.init(resource, ());
	}
}

impl Dispatch<WlDataDeviceManager, (), WaylandState> for WaylandState {
	fn request(
		_state: &mut WaylandState,
		_client: &Client,
		_resource: &WlDataDeviceManager,
		request: <WlDataDeviceManager as Resource>::Request,
		_data: &(),
		_dhandle: &DisplayHandle,
		data_init: &mut DataInit<'_, WaylandState>,
	) {
		match request {
			CreateDataSource { id } => {
				data_init.init(id, ());
			}
			GetDataDevice { id, seat: _ } => {
				data_init.init(id, ());
			}
			_ => unreachable!(),
		}
	}
}

impl Dispatch<WlDataSource, (), WaylandState> for WaylandState {
	fn request(
		_state: &mut WaylandState,
		_client: &Client,
		_resource: &WlDataSource,
		request: <WlDataSource as Resource>::Request,
		_data: &(),
		_dhandle: &DisplayHandle,
		_data_init: &mut DataInit<'_, WaylandState>,
	) {
		match request {
			Offer { mime_type: _ } => {}
			Destroy => {}
			SetActions { dnd_actions: _ } => {}
			_ => unreachable!(),
		}
	}
}

impl Dispatch<WlDataDevice, (), WaylandState> for WaylandState {
	fn request(
		_state: &mut WaylandState,
		_client: &Client,
		_resource: &WlDataDevice,
		request: <WlDataDevice as Resource>::Request,
		_data: &(),
		_dhandle: &DisplayHandle,
		_data_init: &mut DataInit<'_, WaylandState>,
	) {
		match request {
			StartDrag {
				source: _,
				origin: _,
				icon: _,
				serial: _,
			} => {}
			SetSelection {
				source: _,
				serial: _,
			} => {}
			Release => {}
			_ => unreachable!(),
		}
	}
}

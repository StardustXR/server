use super::WaylandState;
use nanoid::nanoid;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use smithay::reexports::wayland_server::{
	backend::ClientId,
	delegate_dispatch, delegate_global_dispatch,
	protocol::{
		wl_keyboard::{self, WlKeyboard},
		wl_pointer::{self, WlPointer},
		wl_seat::{self, Capability, WlSeat, EVT_NAME_SINCE},
		wl_touch::{self, WlTouch},
	},
	Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use std::ops::Deref;
use std::sync::Arc;

pub struct SeatDelegate;

#[derive(Clone)]
pub struct SeatData(Arc<SeatDataInner>);
impl SeatData {
	pub fn new(client: ClientId) -> Self {
		SeatData(Arc::new(SeatDataInner {
			client,
			pointer: OnceCell::new(),
			pointer_active: Mutex::new(false),
			keyboard: OnceCell::new(),
			keyboard_active: Mutex::new(false),
			touch: OnceCell::new(),
		}))
	}
}
impl Deref for SeatData {
	type Target = SeatDataInner;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}

pub struct SeatDataInner {
	client: ClientId,
	pointer: OnceCell<WlPointer>,
	pub pointer_active: Mutex<bool>,
	keyboard: OnceCell<WlKeyboard>,
	pub keyboard_active: Mutex<bool>,
	touch: OnceCell<WlTouch>,
}
impl SeatDataInner {
	pub fn pointer(&self) -> Option<&WlPointer> {
		self.pointer.get()
	}
	pub fn keyboard(&self) -> Option<&WlKeyboard> {
		self.keyboard.get()
	}
	pub fn touch(&self) -> Option<&WlTouch> {
		self.touch.get()
	}
}

impl GlobalDispatch<WlSeat, SeatData, WaylandState> for SeatDelegate {
	fn bind(
		_state: &mut WaylandState,
		_handle: &DisplayHandle,
		_client: &Client,
		resource: New<WlSeat>,
		data: &SeatData,
		data_init: &mut DataInit<'_, WaylandState>,
	) {
		let resource = data_init.init(resource, data.clone());

		if resource.version() >= EVT_NAME_SINCE {
			resource.name(nanoid!());
		}

		resource.capabilities(Capability::Pointer | Capability::Keyboard);
	}

	fn can_view(client: Client, data: &SeatData) -> bool {
		client.id() == data.0.client
	}
}
delegate_global_dispatch!(WaylandState: [WlSeat: SeatData] => SeatDelegate);

impl Dispatch<WlSeat, SeatData, WaylandState> for SeatDelegate {
	fn request(
		_state: &mut WaylandState,
		_client: &Client,
		_resource: &WlSeat,
		request: <WlSeat as Resource>::Request,
		data: &SeatData,
		_dh: &DisplayHandle,
		data_init: &mut DataInit<'_, WaylandState>,
	) {
		match request {
			wl_seat::Request::GetPointer { id } => {
				let _ = data.0.pointer.set(data_init.init(id, data.clone()));
			}
			wl_seat::Request::GetKeyboard { id } => {
				let _ = data.0.keyboard.set(data_init.init(id, data.clone()));
			}
			wl_seat::Request::GetTouch { id } => {
				let _ = data.0.touch.set(data_init.init(id, data.clone()));
			}
			wl_seat::Request::Release => (),
			_ => unreachable!(),
		}
	}
}
delegate_dispatch!(WaylandState: [WlSeat: SeatData] => SeatDelegate);

impl Dispatch<WlPointer, SeatData, WaylandState> for SeatDelegate {
	fn request(
		_state: &mut WaylandState,
		_client: &Client,
		_resource: &WlPointer,
		request: <WlPointer as Resource>::Request,
		_data: &SeatData,
		_dh: &DisplayHandle,
		_data_init: &mut DataInit<'_, WaylandState>,
	) {
		match request {
			wl_pointer::Request::SetCursor {
				serial: _,
				surface: _,
				hotspot_x: _,
				hotspot_y: _,
			} => (),
			wl_pointer::Request::Release => (),
			_ => unreachable!(),
		}
	}
}
delegate_dispatch!(WaylandState: [WlPointer: SeatData] => SeatDelegate);

impl Dispatch<WlKeyboard, SeatData, WaylandState> for SeatDelegate {
	fn request(
		_state: &mut WaylandState,
		_client: &Client,
		_resource: &WlKeyboard,
		request: <WlKeyboard as Resource>::Request,
		_data: &SeatData,
		_dh: &DisplayHandle,
		_data_init: &mut DataInit<'_, WaylandState>,
	) {
		match request {
			wl_keyboard::Request::Release => (),
			_ => unreachable!(),
		}
	}
}
delegate_dispatch!(WaylandState: [WlKeyboard: SeatData] => SeatDelegate);

impl Dispatch<WlTouch, SeatData, WaylandState> for SeatDelegate {
	fn request(
		_state: &mut WaylandState,
		_client: &Client,
		_resource: &WlTouch,
		request: <WlTouch as Resource>::Request,
		_data: &SeatData,
		_dh: &DisplayHandle,
		_data_init: &mut DataInit<'_, WaylandState>,
	) {
		match request {
			wl_touch::Request::Release => (),
			_ => unreachable!(),
		}
	}
}
delegate_dispatch!(WaylandState: [WlTouch: SeatData] => SeatDelegate);

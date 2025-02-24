// SPDX-License-Identifier: GPL-3.0-only

// Re-export only the actual code, and then only use this re-export
// The `generated` module below is just some boilerplate to properly isolate stuff
// and avoid exposing internal details.
//
// You can use all the types from my_protocol as if they went from `wayland_client::protocol`.
pub use generated::wl_drm;

#[allow(non_upper_case_globals, non_camel_case_types)]
mod generated {
	use smithay::reexports::wayland_server::{self, protocol::*};

	pub mod __interfaces {
		use smithay::reexports::wayland_server::protocol::__interfaces::*;
		wayland_scanner::generate_interfaces!("src/wayland/wayland-drm.xml");
	}
	use self::__interfaces::*;

	wayland_scanner::generate_server_code!("src/wayland/wayland-drm.xml");
}

use super::state::WaylandState;
use smithay::{
	backend::allocator::{
		Fourcc, Modifier,
		dmabuf::{Dmabuf, DmabufFlags},
	},
	reexports::wayland_server::{
		Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
	},
};
use std::convert::TryFrom;

impl GlobalDispatch<wl_drm::WlDrm, (), WaylandState> for WaylandState {
	fn bind(
		state: &mut WaylandState,
		_dh: &DisplayHandle,
		_client: &Client,
		resource: New<wl_drm::WlDrm>,
		_global_data: &(),
		data_init: &mut DataInit<'_, WaylandState>,
	) {
		let drm_instance = data_init.init(resource, ());

		drm_instance.device("/dev/dri/renderD128".to_string());
		if drm_instance.version() >= 2 {
			drm_instance.capabilities(wl_drm::Capability::Prime as u32);
		}
		for format in state.drm_formats.iter() {
			if let Ok(converted) = wl_drm::Format::try_from(*format as u32) {
				drm_instance.format(converted as u32);
			}
		}
	}

	fn can_view(_client: Client, _global_dataa: &()) -> bool {
		true
	}
}

impl Dispatch<wl_drm::WlDrm, (), WaylandState> for WaylandState {
	fn request(
		state: &mut WaylandState,
		_client: &Client,
		drm: &wl_drm::WlDrm,
		request: wl_drm::Request,
		_data: &(),
		_dh: &DisplayHandle,
		data_init: &mut DataInit<'_, WaylandState>,
	) {
		match request {
			wl_drm::Request::Authenticate { .. } => drm.authenticated(),
			wl_drm::Request::CreateBuffer { .. } => drm.post_error(
				wl_drm::Error::InvalidName,
				String::from("Flink handles are unsupported, use PRIME"),
			),
			wl_drm::Request::CreatePlanarBuffer { .. } => drm.post_error(
				wl_drm::Error::InvalidName,
				String::from("Flink handles are unsupported, use PRIME"),
			),
			wl_drm::Request::CreatePrimeBuffer {
				id,
				name,
				width,
				height,
				format,
				offset0,
				stride0,
				..
			} => {
				let format = match Fourcc::try_from(format) {
					Ok(format) => {
						if !state.drm_formats.contains(&format) {
							drm.post_error(
								wl_drm::Error::InvalidFormat,
								String::from("Format not advertised by wl_drm"),
							);
							return;
						}
						format
					}
					Err(_) => {
						drm.post_error(
							wl_drm::Error::InvalidFormat,
							String::from("Format unknown / not advertised by wl_drm"),
						);
						return;
					}
				};

				if width < 1 || height < 1 {
					drm.post_error(
						wl_drm::Error::InvalidFormat,
						String::from("width or height not positive"),
					);
					return;
				}

				let mut dma = Dmabuf::builder(
					(width, height),
					format,
					Modifier::Invalid,
					DmabufFlags::empty(),
				);
				dma.add_plane(name, 0, offset0 as u32, stride0 as u32);
				match dma.build() {
					Some(dmabuf) => {
						state.dmabuf_tx.send((dmabuf.clone(), None)).unwrap();
						data_init.init(id, dmabuf);
					}
					None => {
						// Buffer import failed. The protocol documentation heavily implies killing the
						// client is the right thing to do here.
						drm.post_error(
							wl_drm::Error::InvalidName,
							"dmabuf global was destroyed on server",
						);
					}
				}
			}
		}
	}
}

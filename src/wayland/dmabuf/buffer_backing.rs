use super::buffer_params::BufferParams;
use crate::wayland::{Message, MessageSink, core::buffer::Buffer};
use bevy::{
	asset::{Assets, Handle},
	image::Image,
};
use bevy_dmabuf::{
	dmatex::{Dmatex, DmatexPlane, Resolution},
	import::{ImportedDmatexs, ImportedTexture},
};
use drm_fourcc::DrmFourcc;
use mint::Vector2;
use parking_lot::Mutex;
use std::sync::{Arc, OnceLock};
use waynest::server::protocol::stable::linux_dmabuf_v1::zwp_linux_buffer_params_v1::Flags;

/// Parameters for a shared memory buffer
pub struct DmabufBacking {
	message_sink: Option<MessageSink>,
	params: Arc<BufferParams>,
	size: Vector2<u32>,
	format: DrmFourcc,
	_flags: Flags,
	tex: OnceLock<Handle<Image>>,
	pending_imported_dmatex: Mutex<Option<ImportedTexture>>,
}

impl std::fmt::Debug for DmabufBacking {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("DmabufBacking")
			.field("message_sink", &self.message_sink)
			.field("params", &self.params)
			.field("size", &self.size)
			.field("format", &self.format)
			.field("_flags", &self._flags)
			.field("tex", &self.tex)
			.finish()
	}
}

impl DmabufBacking {
	pub fn new(
		params: Arc<BufferParams>,
		message_sink: Option<MessageSink>,
		size: Vector2<u32>,
		format: DrmFourcc,
		flags: Flags,
	) -> Self {
		tracing::info!("Creating new DmabufBacking",);
		Self {
			params,
			message_sink,
			size,
			format,
			_flags: flags,
			tex: OnceLock::new(),
			pending_imported_dmatex: Mutex::new(None),
		}
	}

	fn import_dmabuf(
		&self,
		dmatexes: &ImportedDmatexs,
		images: &mut Assets<Image>,
		buffer: Arc<Buffer>,
	) -> Option<Handle<Image>> {
		let mut planes = std::mem::take(&mut *self.params.planes.lock());
		// TODO: AAAAAA BAD HACK WHAT THE HELL FIX THIS
		let key = *planes.keys().last().unwrap();
		let plane = planes.remove(&key).unwrap();
		let dmatex = Dmatex {
			dmabuf_fd: plane.fd.into(),
			planes: vec![DmatexPlane {
				offset: plane.offset,
				stride: plane.stride as i32,
			}],
			res: Resolution {
				x: self.size.x,
				y: self.size.y,
			},
			modifier: plane.modifier,
			format: self.format as u32,
			flip_y: self._flags.contains(Flags::YInvert),
		};
		let dmatex = dmatexes.set(images, dmatex, None);
		match &dmatex {
			Ok(_) => {
				let _ = self
					.message_sink
					.as_ref()
					.unwrap()
					.send(Message::DmabufImportSuccess(self.params.clone(), buffer));
			}
			Err(e) => {
				tracing::error!("Failed to import dmabuf because {e}");

				let _ = self
					.message_sink
					.as_ref()
					.unwrap()
					.send(Message::DmabufImportFailure(self.params.clone()));
			}
		}

		dmatex.ok()
	}

	pub fn update_tex(
		&self,
		dmatexes: &ImportedDmatexs,
		images: &mut Assets<Image>,
		buffer: Arc<Buffer>,
	) -> Option<Handle<Image>> {
		self.pending_imported_dmatex
			.lock()
			.take()
			.map(|tex| dmatexes.insert_imported_dmatex(images, tex))
		// if self.tex.get().is_none()
		// 	&& let Some(dmatex) = self.import_dmabuf(dmatexes, images, buffer)
		// {
		// 	let _ = self.tex.set(dmatex);
		// }
		// self.tex.get().cloned()
	}

	pub fn is_transparent(&self) -> bool {
		matches!(
			self.format,
			DrmFourcc::Abgr1555
				| DrmFourcc::Abgr16161616f
				| DrmFourcc::Abgr2101010
				| DrmFourcc::Abgr4444
				| DrmFourcc::Abgr8888
				| DrmFourcc::Argb1555
				| DrmFourcc::Argb16161616f
				| DrmFourcc::Argb2101010
				| DrmFourcc::Argb4444
				| DrmFourcc::Argb8888
				| DrmFourcc::Axbxgxrx106106106106
				| DrmFourcc::Ayuv
				| DrmFourcc::Rgba1010102
				| DrmFourcc::Rgba4444
				| DrmFourcc::Rgba5551
				| DrmFourcc::Rgba8888
		)
	}

	pub fn size(&self) -> Vector2<usize> {
		[self.size.x as usize, self.size.y as usize].into()
	}
}

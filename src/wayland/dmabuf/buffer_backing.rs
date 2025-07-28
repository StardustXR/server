use super::buffer_params::BufferParams;
use crate::wayland::RENDER_DEVICE;
use bevy::{
	asset::{Assets, Handle},
	image::Image,
};
use bevy_dmabuf::{
	dmatex::{Dmatex, Resolution},
	import::{DropCallback, ImportError, ImportedDmatexs, ImportedTexture, import_texture},
};
use drm_fourcc::DrmFourcc;
use mint::Vector2;
use parking_lot::Mutex;
use std::sync::{Arc, OnceLock};
use tracing::info;
use waynest::server::protocol::stable::linux_dmabuf_v1::zwp_linux_buffer_params_v1::Flags;

/// Parameters for a shared memory buffer
pub struct DmabufBacking {
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
			.field("params", &self.params)
			.field("size", &self.size)
			.field("format", &self.format)
			.field("_flags", &self._flags)
			.field("tex", &self.tex)
			.finish()
	}
}

impl DmabufBacking {
	#[tracing::instrument(level = "debug", skip_all)]
	pub fn new(
		params: Arc<BufferParams>,
		size: Vector2<u32>,
		format: DrmFourcc,
		flags: Flags,
	) -> Result<Self, ImportError> {
		tracing::info!("Creating new DmabufBacking");
		let mut planes = Vec::from_iter(std::mem::take(&mut *params.planes.lock()));
		planes.sort_by_key(|(index, _)| *index);
		let planes = planes.into_iter().map(|(_, tex)| tex).collect::<Vec<_>>();
		let dmatex = Dmatex {
			planes,
			res: Resolution {
				x: size.x,
				y: size.y,
			},
			format: format as u32,
			// TODO: impl this in bevy-dmabuf
			flip_y: flags.contains(Flags::YInvert),
			srgb: true,
		};
		let dev = RENDER_DEVICE.wait();
		let imported_tex = import_texture(dev, dmatex, DropCallback(None))?;

		Ok(Self {
			params,
			size,
			format,
			_flags: flags,
			tex: OnceLock::new(),
			pending_imported_dmatex: Mutex::new(Some(imported_tex)),
		})
	}

	#[tracing::instrument(level = "debug", skip_all)]
	pub fn update_tex(
		&self,
		dmatexes: &ImportedDmatexs,
		images: &mut Assets<Image>,
	) -> Option<Handle<Image>> {
		info!("updating dmabuf tex");
		self.pending_imported_dmatex
			.lock()
			.take()
			.map(|tex| dmatexes.insert_imported_dmatex(images, tex))
			.inspect(|handle| {
				_ = self.tex.set(handle.clone());
			})
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

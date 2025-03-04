use super::buffer_params::BufferParams;
use crate::wayland::{GraphicsInfo, Message, MessageSink, core::buffer::Buffer};
use drm_fourcc::DrmFourcc;
use khronos_egl::{self as egl, ClientBuffer};
use mint::Vector2;
use std::{
	os::fd::AsRawFd,
	sync::{Arc, OnceLock},
};
use stereokit_rust::tex::{Tex, TexFormat, TexType};
use waynest::server::protocol::stable::linux_dmabuf_v1::zwp_linux_buffer_params_v1::Flags;

// EGL extension constants for DMA-BUF
const EGL_WIDTH: i32 = 0x3057;
const EGL_HEIGHT: i32 = 0x3056;
const EGL_LINUX_DRM_FOURCC_EXT: i32 = 0x3272;
const EGL_DMA_BUF_PLANE0_FD_EXT: i32 = 0x3373;
const EGL_DMA_BUF_PLANE0_OFFSET_EXT: i32 = 0x3273;
const EGL_DMA_BUF_PLANE0_PITCH_EXT: i32 = 0x3274;
const EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT: i32 = 0x3443;
const EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT: i32 = 0x3444;
const EGL_LINUX_DMA_BUF_EXT: i32 = 0x3270;
const EGL_NO_BUFFER: *mut std::ffi::c_void = std::ptr::null_mut();

/// Parameters for a shared memory buffer
#[derive(Debug)]
pub struct DmabufBacking {
	params: Arc<BufferParams>,
	message_sink: Option<MessageSink>,
	size: Vector2<usize>,
	format: DrmFourcc,
	_flags: Flags,
	tex: OnceLock<Tex>,
}

impl DmabufBacking {
	pub fn new(
		params: Arc<BufferParams>,
		message_sink: Option<MessageSink>,
		size: Vector2<usize>,
		format: DrmFourcc,
		flags: Flags,
	) -> Self {
		tracing::info!(
			"Creating new DmabufBacking with BufferParams {:?}",
			params.id
		);
		Self {
			params,
			message_sink,
			size,
			format,
			_flags: flags,
			tex: OnceLock::new(),
		}
	}

	fn import_dmabuf(&self, graphics_info: &GraphicsInfo) -> Result<Tex, khronos_egl::Error> {
		let mut tex = Tex::new(
			TexType::ImageNomips | TexType::Dynamic,
			TexFormat::RGBA32,
			nanoid::nanoid!(),
		);

		tracing::info!(format=?self.format, "Wayland: Updating DMABuf tex");

		// Get plane info from params
		let planes = self.params.lock_planes();
		let Some(plane) = planes.get(&0) else {
			tracing::error!(
				"Wayland: Failed to get plane 0 from BufferParams {:?}",
				self.params.id
			);
			return Err(khronos_egl::Error::BadParameter);
		};
		tracing::info!(
			"Using plane 0 with fd {} from BufferParams {:?}",
			plane.fd.as_raw_fd(),
			self.params.id
		);
		// Create EGL image
		let image = graphics_info.egl_instance.create_image(
			graphics_info.display,
			graphics_info.context,
			EGL_LINUX_DMA_BUF_EXT as u32,
			unsafe { ClientBuffer::from_ptr(EGL_NO_BUFFER) },
			&[
				EGL_WIDTH as usize,
				self.size.x as usize,
				EGL_HEIGHT as usize,
				self.size.y as usize,
				EGL_LINUX_DRM_FOURCC_EXT as usize,
				self.format as usize,
				EGL_DMA_BUF_PLANE0_FD_EXT as usize,
				plane.fd.as_raw_fd() as usize, // EGL will dup() this fd internally
				EGL_DMA_BUF_PLANE0_OFFSET_EXT as usize,
				plane.offset as usize,
				EGL_DMA_BUF_PLANE0_PITCH_EXT as usize,
				plane.stride as usize,
				EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT as usize,
				0,
				EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT as usize,
				0,
				egl::ATTRIB_NONE,
				0,
			],
		)?;

		// The cloned fd will be consumed by create_image, so we don't need to explicitly close it
		// Create and bind GL texture
		let mut gl_tex = 0;
		unsafe {
			gl::GenTextures(1, &mut gl_tex);
			gl::BindTexture(gl::TEXTURE_2D, gl_tex);
		}

		// Set the native texture handle directly
		// Mesa will handle the OES texture implicitly
		tex.set_native_surface(
			gl_tex as *mut std::os::raw::c_void,
			TexType::ImageNomips | TexType::Dynamic,
			0x8058, // GL_RGBA8
			self.size.x as i32,
			self.size.y as i32,
			1,    // single surface
			true, // we own this texture
		);

		// Clean up EGL image
		graphics_info
			.egl_instance
			.destroy_image(graphics_info.display, image)
			.ok();

		Ok(tex)
	}

	pub fn init_tex(&self, graphics_info: &GraphicsInfo, buffer: Arc<Buffer>) {
		if self.tex.get().is_none() {
			match self.import_dmabuf(graphics_info) {
				Ok(tex) => {
					let _ = self.tex.set(tex);
					let _ = self
						.message_sink
						.as_ref()
						.unwrap()
						.send(Message::DmabufImportSuccess(self.params.clone(), buffer));
				}
				Err(e) => {
					tracing::error!("Wayland: Error initializing DMABuf tex: {:?}", e);
					let _ = self
						.message_sink
						.as_ref()
						.unwrap()
						.send(Message::DmabufImportFailure(self.params.clone()));
				}
			};
		}
	}

	pub fn get_tex(&self) -> Option<&Tex> {
		self.tex.get()
	}

	pub fn size(&self) -> Vector2<usize> {
		self.size
	}
}

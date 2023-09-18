use std::ffi::c_void;

use color_eyre::eyre::{ensure, Result};
use smithay::backend::{egl::EGLContext, renderer::gles::GlesRenderer};
use stereokit as sk;

struct EGLRawHandles {
	display: *const c_void,
	config: *const c_void,
	context: *const c_void,
}
fn get_sk_egl() -> Result<EGLRawHandles> {
	ensure!(
		unsafe { sk::sys::backend_graphics_get() }
			== sk::sys::backend_graphics__backend_graphics_opengles_egl,
		"StereoKit is not running using EGL!"
	);

	Ok(unsafe {
		EGLRawHandles {
			display: sk::sys::backend_opengl_egl_get_display() as *const c_void,
			config: sk::sys::backend_opengl_egl_get_config() as *const c_void,
			context: sk::sys::backend_opengl_egl_get_context() as *const c_void,
		}
	})
}

pub struct BufferManager {
	pub renderer: GlesRenderer,
}
impl BufferManager {
	pub fn new() -> Result<Self> {
		let egl_raw_handles = get_sk_egl()?;
		let renderer = unsafe {
			GlesRenderer::new(EGLContext::from_raw(
				egl_raw_handles.display,
				egl_raw_handles.config,
				egl_raw_handles.context,
			)?)?
		};

		Ok(BufferManager { renderer })
	}

	pub fn make_context_current(&self) {
		unsafe {
			let _ = self.renderer.egl_context().make_current();
		}
	}
}

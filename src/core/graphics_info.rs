use khronos_egl::{Context, Display, Instance};
use std::ffi::CStr;
use waynest::server::Error;

// Define function pointers for the missing EGL extension functions
type EglQueryDisplayAttribEXT = unsafe extern "C" fn(
	egl_display: khronos_egl::EGLDisplay,
	attribute: i32,
	value: *mut *const std::ffi::c_void,
) -> khronos_egl::Boolean;

type EglQueryDeviceStringEXT =
	unsafe extern "C" fn(device: *const std::ffi::c_void, name: i32) -> *const i8;

#[derive(Debug)]
pub struct GraphicsInfo {
	pub egl_instance: Instance<khronos_egl::Static>,
	pub display: Display,
	pub context: Context,
}
unsafe impl Send for GraphicsInfo {}
unsafe impl Sync for GraphicsInfo {}

impl GraphicsInfo {
	pub fn get_drm_device_file_path(&self) -> Result<String, Error> {
		// Load the missing EGL extension functions
		let egl_query_display_attrib_ext = {
			let func = self
				.egl_instance
				.get_proc_address("eglQueryDisplayAttribEXT");
			let func =
				func.ok_or_else(|| Error::Custom("eglQueryDisplayAttribEXT not available".into()))?;
			unsafe { std::mem::transmute::<extern "system" fn(), EglQueryDisplayAttribEXT>(func) }
		};

		let egl_query_device_string_ext = {
			let func = self
				.egl_instance
				.get_proc_address("eglQueryDeviceStringEXT");
			let func =
				func.ok_or_else(|| Error::Custom("eglQueryDeviceStringEXT not available".into()))?;
			unsafe { std::mem::transmute::<extern "system" fn(), EglQueryDeviceStringEXT>(func) }
		};

		// Query the EGL device
		let egl_display = self.display;
		let egl_attributes: i32 = 0x322C; // EGL_DEVICE_EXT
		let egl_device = {
			let mut device: *const std::ffi::c_void = std::ptr::null_mut();
			let success = unsafe {
				egl_query_display_attrib_ext(egl_display.as_ptr(), egl_attributes, &mut device)
			};
			if success == khronos_egl::FALSE {
				return Err(Error::Custom("Failed to query EGL device".into()));
			}
			device
		};

		// Get the DRM device file path
		let drm_device_file_path = unsafe {
			let cstr = egl_query_device_string_ext(egl_device, 0x3376); // EGL_DRM_DEVICE_FILE_EXT
			if cstr.is_null() {
				return Err(Error::Custom("Failed to query DRM device file path".into()));
			}
			CStr::from_ptr(cstr)
				.to_str()
				.map_err(|_| {
					Error::Custom("Failed to convert DRM device file path to string".into())
				})?
				.to_string()
		};

		Ok(drm_device_file_path)
	}
}

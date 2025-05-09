use khronos_egl::{Context, Display, Instance};
use std::ffi::CStr;
use std::io::{self, Error};

#[derive(Debug)]
pub struct GraphicsInfo {
	pub instance: Instance<khronos_egl::Static>,
	pub display: Display,
	pub context: Context,
}
unsafe impl Send for GraphicsInfo {}
unsafe impl Sync for GraphicsInfo {}

impl GraphicsInfo {
	#[allow(unused)]
	pub fn get_drm_device_file_path(&self) -> Result<String, Error> {
		// Define function pointers for the missing EGL extension functions
		type EglQueryDisplayAttribEXT = unsafe extern "C" fn(
			egl_display: khronos_egl::EGLDisplay,
			attribute: i32,
			value: *mut *const std::ffi::c_void,
		) -> khronos_egl::Boolean;

		type EglQueryDeviceStringEXT =
			unsafe extern "C" fn(device: *const std::ffi::c_void, name: i32) -> *const i8;

		// Load the missing EGL extension functions
		let egl_query_display_attrib_ext = {
			let func = self.instance.get_proc_address("eglQueryDisplayAttribEXT");
			let func = func.ok_or_else(|| {
				io::Error::new(
					io::ErrorKind::Other,
					"eglQueryDisplayAttribEXT not available",
				)
			})?;
			unsafe { std::mem::transmute::<extern "system" fn(), EglQueryDisplayAttribEXT>(func) }
		};

		let egl_query_device_string_ext = {
			let func = self.instance.get_proc_address("eglQueryDeviceStringEXT");
			let func = func.ok_or_else(|| {
				io::Error::new(
					io::ErrorKind::Other,
					"eglQueryDeviceStringEXT not available",
				)
			})?;
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
			if device.is_null() {
				return Err(io::Error::new(io::ErrorKind::Other, "egl_device is null"));
			}
			if success == khronos_egl::FALSE {
				let egl_error = self.instance.get_error();
				return Err(io::Error::new(
					io::ErrorKind::Other,
					format!("Failed to query EGL device: {:?}", egl_error),
				));
			}
			device
		};

		// Check supported attributes for the device
		const EGL_EXTENSIONS: i32 = 0x3055;
		let supported_attributes =
			unsafe { egl_query_device_string_ext(egl_device, EGL_EXTENSIONS) };
		if supported_attributes.is_null() {
			return Err(io::Error::new(
				io::ErrorKind::Other,
				"Failed to query supported attributes",
			));
		}
		let extensions = unsafe { CStr::from_ptr(supported_attributes).to_str().unwrap_or("") };
		if !extensions.contains("EGL_DRM_DEVICE_FILE_EXT") {
			return Err(io::Error::new(
				io::ErrorKind::Other,
				"EGL_DRM_DEVICE_FILE_EXT not supported",
			));
		}

		// Ensure EGL context is current
		let previous_context = self.instance.get_current_context();
		let previous_display = self.instance.get_current_display();
		self.instance
			.make_current(self.display, None, None, Some(self.context))
			.map_err(|_| {
				io::Error::new(io::ErrorKind::Other, "Failed to make EGL context current")
			})?;
		let drm_device_file_path = unsafe {
			let cstr = egl_query_device_string_ext(egl_device, 0x3376); // EGL_DRM_DEVICE_FILE_EXT
			if cstr.is_null() {
				let egl_error = self.instance.get_error();
				return Err(io::Error::new(
					io::ErrorKind::Other,
					format!("Failed to query DRM device file path: {:?}", egl_error),
				));
			}
			CStr::from_ptr(cstr)
				.to_str()
				.map_err(|e| {
					io::Error::new(
						io::ErrorKind::InvalidData,
						format!("Failed to convert DRM device file path to string: {}", e),
					)
				})?
				.to_string()
		};

		// Restore previous EGL context state
		if let Some(previous_display) = previous_display {
			self.instance
				.make_current(previous_display, None, None, previous_context)
				.map_err(|e| {
					let egl_error = self.instance.get_error();
					io::Error::new(
						io::ErrorKind::Other,
						format!(
							"Failed to restore previous EGL context state: {:?}, EGL error: {:?}",
							e, egl_error
						),
					)
				})?;
		}

		Ok(drm_device_file_path)
	}
}

#[test]
fn test_get_drm_device_file_path() {
	use std::sync::Arc;
	use stereokit_rust::sk::AppMode;
	use stereokit_rust::sk::SkSettings;
	use stereokit_rust::system::BackendOpenGLESEGL;

	// Initialize StereoKit
	let sk = SkSettings::default()
		.app_name("GraphicsInfo Test")
		.mode(if std::env::args().any(|arg| arg == "-f") {
			AppMode::Simulator
		} else {
			AppMode::XR
		})
		.init()
		.expect("StereoKit failed to initialize");

	// Create GraphicsInfo instance
	let graphics_info = unsafe {
		Arc::new(GraphicsInfo {
			instance: khronos_egl::Instance::new(khronos_egl::Static),
			display: khronos_egl::Display::from_ptr(BackendOpenGLESEGL::display()),
			context: khronos_egl::Context::from_ptr(BackendOpenGLESEGL::context()),
		})
	};

	sk.step();

	// Call get_drm_device_file_path and log the result
	match graphics_info.get_drm_device_file_path() {
		Ok(path) => println!("DRM Device File Path: {}", path),
		Err(e) => eprintln!("Error retrieving DRM Device File Path: {:?}", e),
	}

	std::io::Write::flush(&mut std::io::stdout()).expect("Failed to flush stdout");
}

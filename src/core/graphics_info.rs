use khronos_egl::{Context, Display, Instance};
use std::ffi::CStr;
use std::io::{self, Error};

const EGL_EXTENSIONS: i32 = 0x3055;

// Define function pointers for the missing EGL extension functions
type EglQueryDisplayAttribEXT = unsafe extern "C" fn(
	egl_display: khronos_egl::EGLDisplay,
	attribute: i32,
	value: *mut *const std::ffi::c_void,
) -> khronos_egl::Boolean;

type EglQueryDeviceStringEXT =
	unsafe extern "C" fn(device: *const std::ffi::c_void, name: i32) -> *const i8;

extern "C" fn egl_debug_callback(
	error: i32,
	command: *const std::os::raw::c_char,
	message_type: std::os::raw::c_int,
	thread: std::os::raw::c_void,
	object: std::os::raw::c_void,
	message: *const std::os::raw::c_char,
) {
	let command = unsafe { std::ffi::CStr::from_ptr(command) }.to_string_lossy();
	let message = unsafe { std::ffi::CStr::from_ptr(message) }.to_string_lossy();
	eprintln!(
		"EGL Debug: Error: {:?}, Command: {}, Message Type: {}, Thread: {:?}, Object: {:?}, Message: {}",
		error, command, message_type, thread, object, message
	);
}

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
	pub fn register_debug_callback(&self) -> Result<(), Error> {
		let extensions_str = self.extensions()?;
		dbg!(&extensions_str);
		if extensions_str.contains("EGL_KHR_debug") {
			let egl_debug_message_control_khr = {
				let func = self.instance.get_proc_address("eglDebugMessageControlKHR");
				let func = func.ok_or_else(|| {
					io::Error::new(
						io::ErrorKind::Other,
						"eglDebugMessageControlKHR not available",
					)
				})?;
				unsafe {
					std::mem::transmute::<
						extern "system" fn(),
						unsafe extern "C" fn(
							callback: Option<
								extern "C" fn(
									error: i32,
									command: *const std::os::raw::c_char,
									message_type: std::os::raw::c_int,
									thread: std::os::raw::c_void,
									object: std::os::raw::c_void,
									message: *const std::os::raw::c_char,
								),
							>,
							*const std::ffi::c_void,
						) -> khronos_egl::Boolean,
					>(func)
				}
			};

			unsafe {
				egl_debug_message_control_khr(Some(egl_debug_callback), std::ptr::null());
			}
			Ok(())
		} else {
			eprintln!("EGL_KHR_debug extension is not supported");
			Err(io::Error::new(
				io::ErrorKind::Other,
				"EGL_KHR_debug extension is not supported",
			))
		}
	}

	pub fn extensions(&self) -> Result<String, Error> {
		let egl_device = self.egl_device()?;

		// Check supported attributes for the device
		let supported_attributes =
			unsafe { self.egl_query_device_string_ext()?(egl_device, EGL_EXTENSIONS) };
		if supported_attributes.is_null() {
			return Err(io::Error::new(
				io::ErrorKind::Other,
				"Failed to query supported attributes",
			));
		}
		Ok(unsafe {
			CStr::from_ptr(supported_attributes)
				.to_string_lossy()
				.to_string()
		})
	}

	fn egl_device(&self) -> Result<*const std::ffi::c_void, Error> {
		let egl_display = self.display;
		let egl_attributes: i32 = 0x322C;
		let mut device: *const std::ffi::c_void = std::ptr::null_mut();
		let success = unsafe {
			self.egl_query_display_attrib_ext()?(egl_display.as_ptr(), egl_attributes, &mut device)
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

		Ok(device)
	}

	fn egl_query_display_attrib_ext(&self) -> Result<EglQueryDisplayAttribEXT, Error> {
		let func = self.instance.get_proc_address("eglQueryDisplayAttribEXT");
		let func = func.ok_or_else(|| {
			io::Error::new(
				io::ErrorKind::Other,
				"eglQueryDisplayAttribEXT not available",
			)
		})?;
		unsafe {
			Ok(std::mem::transmute::<
				extern "system" fn(),
				EglQueryDisplayAttribEXT,
			>(func))
		}
	}

	fn egl_query_device_string_ext(&self) -> Result<EglQueryDeviceStringEXT, Error> {
		let func = self.instance.get_proc_address("eglQueryDeviceStringEXT");
		let func = func.ok_or_else(|| {
			io::Error::new(
				io::ErrorKind::Other,
				"eglQueryDeviceStringEXT not available",
			)
		})?;
		unsafe {
			Ok(std::mem::transmute::<
				extern "system" fn(),
				EglQueryDeviceStringEXT,
			>(func))
		}
	}

	#[allow(unused)]
	pub fn get_drm_device_file_path(&self) -> Result<String, Error> {
		let extensions = self.extensions()?;
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
			let cstr = self.egl_query_device_string_ext()?(self.egl_device()?, 0x3376); // EGL_DRM_DEVICE_FILE_EXT
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

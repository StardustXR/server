mod core;
mod nodes;
mod wayland;

use crate::nodes::model::{MODELS_TO_DROP, MODEL_REGISTRY};
use crate::wayland::{ClientState, WaylandState};

use self::core::eventloop::EventLoop;
use anyhow::{ensure, Result};
use clap::Parser;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use slog::Drain;
use smithay::backend::egl::EGLContext;
use smithay::backend::renderer::gles2::Gles2Renderer;
use smithay::reexports::wayland_server::{Display, ListeningSocket};
use std::ffi::c_void;
use std::sync::Arc;
use stereokit as sk;
use stereokit::{lifecycle::DisplayMode, Settings};
use tokio::{runtime::Handle, sync::oneshot};

static TOKIO_HANDLE: Lazy<Mutex<Option<Handle>>> = Lazy::new(Default::default);

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
struct CliArgs {
	/// Force flatscreen mode and use the mouse pointer as a 3D pointer
	#[clap(short, action)]
	flatscreen: bool,

	/// Run Stardust XR as an overlay
	#[clap(short, action)]
	overlay: bool,
}

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

fn main() -> Result<()> {
	let cli_args = Arc::new(CliArgs::parse());
	let log = ::slog::Logger::root(::slog_stdlog::StdLog.fuse(), slog::o!());
	slog_stdlog::init()?;

	let stereokit = Settings::default()
		.app_name("Stardust XR")
		.overlay_app(cli_args.overlay)
		.overlay_priority(u32::MAX)
		.disable_desktop_input_window(true)
		.display_preference(if cli_args.flatscreen {
			DisplayMode::Flatscreen
		} else {
			DisplayMode::MixedReality
		})
		.init()
		.expect("StereoKit failed to initialize");

	let (event_stop_tx, event_stop_rx) = oneshot::channel::<()>();
	let event_thread = std::thread::Builder::new()
		.name("event_loop".to_owned())
		.spawn(move || event_loop(event_stop_rx))?;

	let egl_raw_handles = get_sk_egl()?;
	let renderer = unsafe {
		Gles2Renderer::new(
			EGLContext::from_raw(
				egl_raw_handles.display,
				egl_raw_handles.config,
				egl_raw_handles.context,
				log.clone(),
			)?,
			log.clone(),
		)?
	};

	let mut display: Display<WaylandState> = Display::new()?;
	let socket = ListeningSocket::bind_auto("wayland", 0..33)?;
	if let Some(socket_name) = socket.socket_name() {
		println!("Wayland compositor {:?} active", socket_name);
	}
	let mut wayland_state = WaylandState::new(&display, renderer, log)?;

	stereokit.run(
		|draw_ctx| {
			if let Ok(Some(client)) = socket.accept() {
				let _ = display
					.handle()
					.insert_client(client, Arc::new(ClientState));
			}
			display.dispatch_clients(&mut wayland_state).unwrap();
			display.flush_clients().unwrap();

			nodes::root::Root::logic_step(stereokit.time_elapsed());
			for model in MODEL_REGISTRY.get_valid_contents() {
				model.draw(&stereokit, draw_ctx);
			}
			MODELS_TO_DROP.lock().clear();

			unsafe { wayland_state.renderer.egl_context().make_current().unwrap() };
		},
		|| {
			println!("Cleanly shut down StereoKit");
		},
	);

	drop(wayland_state);
	drop(socket);
	drop(display);
	println!("Cleanly shut down the Wayland compositor");

	let _ = event_stop_tx.send(());
	event_thread
		.join()
		.expect("Failed to cleanly shut down event loop")?;
	println!("Cleanly shut down Stardust");
	Ok(())
}

#[tokio::main]
async fn event_loop(stop_rx: oneshot::Receiver<()>) -> anyhow::Result<()> {
	TOKIO_HANDLE.lock().replace(Handle::current());

	let (event_loop, event_loop_join_handle) =
		EventLoop::new().expect("Couldn't create server socket");
	println!("Init event loop");
	println!("Stardust socket created at {}", event_loop.socket_path);

	let result = tokio::select! {
		biased;
		_ = tokio::signal::ctrl_c() => Ok(()),
		_ = stop_rx => Ok(()),
		e = event_loop_join_handle => e?,
	};

	println!("Cleanly shut down event loop");

	unsafe {
		stereokit::sys::sk_quit();
	}

	result
}

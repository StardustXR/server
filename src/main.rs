mod core;
mod nodes;

use self::core::eventloop::EventLoop;
use anyhow::Result;
use clap::Parser;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::sync::Arc;
use stereokit::{lifecycle::DisplayMode, Settings};
use tokio::runtime::Handle;

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

fn main() -> Result<()> {
	let cli_args = Arc::new(CliArgs::parse());

	let mut init_settings = Settings::default()
		.app_name("Stardust XR")
		.overlay_app(cli_args.overlay)
		.overlay_priority(u32::MAX);
	if cli_args.flatscreen {
		init_settings = init_settings.display_preference(DisplayMode::Flatscreen);
	}
	let stereokit = init_settings
		.init()
		.expect("StereoKit failed to initialize");

	let event_thread = std::thread::Builder::new()
		.name("event_loop".to_owned())
		.spawn(event_loop)?;

	stereokit.run(
		|_draw_ctx| {
			nodes::root::Root::logic_step(stereokit.time_elapsed());
		},
		|| {
			println!("Shut down StereoKit");
		},
	);
	event_thread
		.join()
		.expect("Failed to cleanly shut down event loop")?;
	println!("Cleanly shut down Stardust");
	Ok(())
}

#[tokio::main]
async fn event_loop() -> anyhow::Result<()> {
	TOKIO_HANDLE.lock().replace(Handle::current());

	let (event_loop, event_loop_join_handle) =
		EventLoop::new().expect("Couldn't create server socket");
	println!("Init event loop");
	println!("Stardust socket created at {}", event_loop.socket_path);

	let result = tokio::select! {
		biased;
		_ = tokio::signal::ctrl_c() => Ok(()),
		// e = task => e?,
		e = event_loop_join_handle => e?,
	};

	unsafe {
		stereokit::sys::sk_quit();
	}

	result
}

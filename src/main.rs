mod core;
mod nodes;

use crate::nodes::model::{MODELS_TO_DROP, MODEL_REGISTRY};

use self::core::eventloop::EventLoop;
use anyhow::Result;
use clap::Parser;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::sync::Arc;
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

fn main() -> Result<()> {
	let cli_args = Arc::new(CliArgs::parse());

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

	stereokit.run(
		|draw_ctx| {
			nodes::root::Root::logic_step(stereokit.time_elapsed());
			for model in MODEL_REGISTRY.get_valid_contents() {
				model.draw(&stereokit, draw_ctx);
			}
			MODELS_TO_DROP.lock().clear();
		},
		|| {
			println!("Cleanly shut down StereoKit");
		},
	);

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

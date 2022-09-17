mod core;
mod nodes;
mod objects;
mod wayland;

use crate::core::destroy_queue;
use crate::nodes::model::{MODELS_TO_DROP, MODEL_REGISTRY};
use crate::nodes::{hmd, input};
use crate::objects::input::mouse_pointer::MousePointer;
use crate::objects::input::sk_hand::SkHand;
use crate::wayland::Wayland;

use self::core::eventloop::EventLoop;
use anyhow::Result;
use clap::Parser;
use slog::Drain;
use std::sync::Arc;
use stereokit::input::Handed;
use stereokit::{lifecycle::DisplayMode, Settings};
use tokio::{runtime::Handle, sync::oneshot};

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
	println!("Init StereoKit");

	let mouse_pointer = cli_args.flatscreen.then(MousePointer::new);
	let mut hands =
		(!cli_args.flatscreen).then(|| [SkHand::new(Handed::Left), SkHand::new(Handed::Right)]);

	if hands.is_none() {
		unsafe {
			stereokit::sys::input_hand_visible(stereokit::sys::handed__handed_left, false as i32);
			stereokit::sys::input_hand_visible(stereokit::sys::handed__handed_right, false as i32);
		}
	}

	let (event_stop_tx, event_stop_rx) = oneshot::channel::<()>();
	let (handle_sender, handle_receiver) = oneshot::channel::<Handle>();
	let event_thread = std::thread::Builder::new()
		.name("event_loop".to_owned())
		.spawn(move || event_loop(handle_sender, event_stop_rx))?;
	let _tokio_handle = handle_receiver.blocking_recv()?.enter();

	let mut wayland = Wayland::new(log)?;
	println!("Stardust ready!");
	stereokit.run(
		|draw_ctx| {
			hmd::frame(&stereokit);
			wayland.frame(&stereokit);
			destroy_queue::clear();

			nodes::root::Root::logic_step(stereokit.time_elapsed());
			for model in MODEL_REGISTRY.get_valid_contents() {
				model.draw(&stereokit, draw_ctx);
			}
			MODELS_TO_DROP.lock().clear();

			if let Some(mouse_pointer) = &mouse_pointer {
				mouse_pointer.update(&stereokit);
			}
			if let Some(hands) = &mut hands {
				hands[0].update(&stereokit);
				hands[1].update(&stereokit);
			}
			input::process_input();

			wayland.make_context_current();
		},
		|| {
			println!("Cleanly shut down StereoKit");
		},
	);

	drop(wayland);

	let _ = event_stop_tx.send(());
	event_thread
		.join()
		.expect("Failed to cleanly shut down event loop")?;
	println!("Cleanly shut down Stardust");
	Ok(())
}

// #[tokio::main]
#[tokio::main(flavor = "current_thread")]
async fn event_loop(
	handle_sender: oneshot::Sender<Handle>,
	stop_rx: oneshot::Receiver<()>,
) -> anyhow::Result<()> {
	let _ = handle_sender.send(Handle::current());
	// console_subscriber::init();

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

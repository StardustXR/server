mod core;
mod nodes;

use self::core::eventloop::EventLoop;
use anyhow::{ensure, Result};
use clap::Parser;
use stereokit_rs as sk;
use stereokit_rs::enums::DisplayMode;
use stereokit_rs::functions::*;

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
struct CliArgs {
	/// Force flatscreen mode and use the mouse pointer as a 3D pointer
	#[clap(short, action)]
	flatscreen: bool,
}

fn main() -> Result<()> {
	let cli_args = CliArgs::parse();
	ctrlc::set_handler(sk_quit).expect("Error setting Ctrl-C handler");

	let mut init_settings = SKSettings::default().app_name("Stardust XR");
	if cli_args.flatscreen {
		init_settings = init_settings.display_preference(DisplayMode::Flatscreen);
	}
	ensure!(init_settings.init(), "StereoKit failed to initialize");

	let event_loop = EventLoop::new(None).expect("Couldn't create server socket");
	println!("Stardust socket created at {}", event_loop.socket_path);

	let mut previous_time = 0_f64;
	sk_run(
		&mut Box::new(&mut move || {
			let current_time = unsafe { sk::sys::time_get() };
			nodes::root::Root::logic_step(current_time - previous_time);
			previous_time = current_time;
		}),
		&mut Box::new(&mut || {
			println!("Shutting down...");
		}),
	);

	Ok(())
}

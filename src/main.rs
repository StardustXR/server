mod core;
mod nodes;

use self::core::eventloop::EventLoop;
use stereokit_rs::functions::*;

fn main() {
	ctrlc::set_handler(|| sk_quit()).expect("Error setting Ctrl-C handler");

	SKSettings::default().app_name("Stardust XR").init();

	let event_loop = EventLoop::new(None).expect("Couldn't create server socket");
	println!("Stardust socket created at {}", event_loop.socket_path);

	sk_run_data(
		&mut Box::new(&mut || {
			// println!("hii uwu");
		}),
		&mut Box::new(&mut || {
			println!("Shutting down...");
		}),
	);

	sk_shutdown();
}

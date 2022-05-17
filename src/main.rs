mod core;
mod nodes;
use self::core::eventloop::EventLoop;
use std::sync::mpsc::channel;

fn main() {
	let (tx, rx) = channel();

	ctrlc::set_handler(move || tx.send(()).expect("Could not send signal on channel."))
		.expect("Error setting Ctrl-C handler");

	println!("Setting up Stardust socket...");
	let event_loop = EventLoop::new(None).expect("Couldn't create server socket");
	println!("Stardust socket created at {}", event_loop.socket_path);

	rx.recv().expect("Could not receive from channel.");
}

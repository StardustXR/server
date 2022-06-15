mod core;
mod nodes;
use self::core::eventloop::EventLoop;
use std::sync::mpsc::{channel, TryRecvError};

fn main() {
	let (tx, rx) = channel();

	ctrlc::set_handler(move || tx.send(()).unwrap()).expect("Error setting Ctrl-C handler");

	let event_loop = EventLoop::new(None).expect("Couldn't create server socket");
	println!("Stardust socket created at {}", event_loop.socket_path);

	while let Err(TryRecvError::Empty) = rx.try_recv() {
		std::thread::sleep(std::time::Duration::from_millis(1000 / 60));
	}
}

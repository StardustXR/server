mod core;
use self::core::eventloop::EventLoop;

fn main() {
	println!("Setting up Stardust socket...");
	let event_loop = EventLoop::new().expect("Couldn't create server socket");
	println!("Stardust socket created at {}", event_loop.socket_path);
}

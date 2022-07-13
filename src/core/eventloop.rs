use super::client::Client;
use anyhow::Result;
use libstardustxr::server;
use mio::net::UnixListener;
use mio::unix::pipe;
use mio::{Events, Interest, Poll, Token};
use slab::Slab;
use std::io::Write;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

pub static FRAME: AtomicU64 = AtomicU64::new(0);

pub struct EventLoop {
	pub socket_path: String,
	join_handle: Option<JoinHandle<Result<()>>>,
	stop_write: pipe::Sender,
}

impl EventLoop {
	pub fn new(timeout: Option<core::time::Duration>) -> Result<Self> {
		let socket_path = server::get_free_socket_path()
			.ok_or_else(|| std::io::Error::from(std::io::ErrorKind::Other))?;
		let (sender, receiver) = pipe::new()?;
		let socket_path_captured = socket_path.clone();
		let join_handle = thread::Builder::new()
			.name("event_loop".to_owned())
			.spawn(move || EventLoop::run_loop(timeout, socket_path_captured, receiver))
			.ok();
		Ok(EventLoop {
			socket_path,
			join_handle,
			stop_write: sender,
		})
	}

	fn run_loop(
		timeout: Option<core::time::Duration>,
		socket_path: String,
		stop_receiver: pipe::Receiver,
	) -> Result<()> {
		let mut stop_receiver = stop_receiver;
		let mut socket = UnixListener::bind(socket_path)?;
		let mut clients: Slab<Option<Arc<Client>>> = Slab::new();
		let mut poll = Poll::new()?;
		let mut events = Events::with_capacity(1024);
		const LISTENER: Token = Token(usize::MAX - 1);
		poll.registry()
			.register(&mut socket, LISTENER, Interest::READABLE)?;
		const STOP: Token = Token(usize::MAX);
		poll.registry()
			.register(&mut stop_receiver, STOP, Interest::READABLE)?;
		'event_loop: loop {
			poll.poll(&mut events, timeout)?;
			for event in &events {
				match event.token() {
					LISTENER => EventLoop::accept_client(&socket, &mut clients, &poll)?,
					STOP => break 'event_loop,
					token => EventLoop::handle_client_message(token.0, &mut clients)?,
				}
			}
		}

		println!("Event loop gracefully finished");
		Ok(())
	}

	fn accept_client(
		socket: &UnixListener,
		clients: &mut Slab<Option<Arc<Client>>>,
		poll: &Poll,
	) -> Result<()> {
		loop {
			match socket.accept() {
				Ok((mut socket, _)) => {
					let client_number = clients.insert(None);
					poll.registry().register(
						&mut socket,
						Token(client_number),
						Interest::READABLE,
					)?;
					let client = Client::from_connection(socket);
					*clients.get_mut(client_number).unwrap() = Some(client);
				}
				Err(e) => match e.kind() {
					std::io::ErrorKind::WouldBlock => break,
					_ => return Err(e.into()),
				},
			}
		}
		Ok(())
	}

	fn handle_client_message(
		client_id: usize,
		clients: &mut Slab<Option<Arc<Client>>>,
	) -> Result<()> {
		let client = clients.get(client_id).and_then(|client| client.as_ref());
		if let Some(client) = client {
			loop {
				let dispatch_result = client.dispatch();
				match dispatch_result {
					Ok(_) => continue,
					Err(e) => match e.kind() {
						std::io::ErrorKind::WouldBlock => break,
						std::io::ErrorKind::Interrupted => continue,
						_ => {
							clients.remove(client_id);
							break;
						}
					},
				}
			}
		}
		Ok(())
	}
}

impl Drop for EventLoop {
	fn drop(&mut self) {
		let buf: [u8; 1] = [1; 1];
		let _ = self.stop_write.write(buf.as_slice());
		if let Some(handle) = self.join_handle.take() {
			match handle.join() {
				Ok(r) => {
					if let Err(e) = r {
						eprintln!("Event loop error: {}", e);
					}
				}
				Err(e) => eprintln!("Event loop failed to rejoin with error: {:#?}", e),
			}
		}
	}
}

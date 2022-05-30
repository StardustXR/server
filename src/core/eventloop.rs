use super::client::Client;
use libstardustxr::server;
use mio::net::UnixListener;
use mio::unix::pipe;
use mio::{Events, Interest, Poll, Token};
use rccell::RcCell;
use slab::Slab;
use std::io::Write;
use std::thread::{self, JoinHandle};

use anyhow::Result;

pub struct EventLoop {
	pub socket_path: String,
	join_handle: Option<JoinHandle<Result<()>>>,
	stop_write: pipe::Sender,
}

impl EventLoop {
	pub fn new(timeout: Option<core::time::Duration>) -> Result<Self> {
		let socket_path = server::get_free_socket_path()
			.ok_or_else(|| std::io::Error::from(std::io::ErrorKind::Other))?;
		let mut socket = UnixListener::bind(socket_path.clone())?;
		let (sender, mut receiver) = pipe::new()?;
		let join_handle = Some(thread::spawn(move || -> Result<()> {
			let mut clients: Slab<Option<RcCell<Client>>> = Slab::new();
			let mut poll = Poll::new()?;
			let mut events = Events::with_capacity(1024);
			const LISTENER: Token = Token(usize::MAX - 1);
			poll.registry()
				.register(&mut socket, LISTENER, Interest::READABLE)?;
			const STOP: Token = Token(usize::MAX);
			poll.registry()
				.register(&mut receiver, STOP, Interest::READABLE)?;
			loop {
				poll.poll(&mut events, timeout)?;
				for event in &events {
					match event.token() {
						LISTENER => loop {
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
								Err(e) => {
									if e.kind() == std::io::ErrorKind::WouldBlock {
										break;
									}
									return Err(e.into());
								}
							}
						},
						STOP => return Ok(()),
						token => loop {
							match clients
								.get(token.0)
								.unwrap()
								.as_ref()
								.unwrap()
								.borrow()
								.dispatch()
							{
								Ok(_) => continue,
								Err(e) => {
									if e.kind() == std::io::ErrorKind::WouldBlock {
										break;
									}
									return Err(e.into());
								}
							}
						},
					}
				}
			}
		}));
		Ok(EventLoop {
			socket_path,
			join_handle,
			stop_write: sender,
		})
	}
}

impl Drop for EventLoop {
	fn drop(&mut self) {
		let buf: [u8; 1] = [1; 1];
		let _ = self.stop_write.write(buf.as_slice());
		let _ = self
			.join_handle
			.take()
			.unwrap()
			.join()
			.expect("Couldn't join the event loop thread at drop");
	}
}

use super::client::Client;
use anyhow::Result;
use libstardustxr::server;
use mio::net::UnixListener;
use mio::unix::pipe;
use mio::{Events, Interest, Poll, Token};
use slab::Slab;
use std::io::Write;
use std::rc::Rc;
use std::sync::{Arc, RwLock};
use std::thread::{self, JoinHandle};

pub struct EventLoop {
	pub socket_path: String,
	join_handle: RwLock<Option<JoinHandle<Result<()>>>>,
	stop_write: pipe::Sender,
}

impl EventLoop {
	pub fn new(timeout: Option<core::time::Duration>) -> Result<Arc<Self>> {
		let socket_path = server::get_free_socket_path()
			.ok_or_else(|| std::io::Error::from(std::io::ErrorKind::Other))?;
		let mut socket = UnixListener::bind(socket_path.clone())?;
		let (sender, mut receiver) = pipe::new()?;
		let event_loop_arc = Arc::new(EventLoop {
			socket_path,
			join_handle: RwLock::new(None),
			stop_write: sender,
		});
		let event_loop_arc_captured = event_loop_arc.clone();
		let join_handle = thread::Builder::new()
			.name("event_loop".to_owned())
			.spawn(move || -> Result<()> {
				let event_loop_arc = event_loop_arc_captured;
				let mut clients: Slab<Option<Rc<Client>>> = Slab::new();
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
										let client =
											Client::from_connection(socket, &event_loop_arc);
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
								match clients.get(token.0).unwrap().as_ref().unwrap().dispatch() {
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
			})
			.ok();
		event_loop_arc.set_join_handle(join_handle);
		Ok(event_loop_arc)
	}

	fn set_join_handle(&self, handle: Option<JoinHandle<Result<()>>>) {
		*self.join_handle.write().unwrap() = handle;
	}
}

impl Drop for EventLoop {
	fn drop(&mut self) {
		let buf: [u8; 1] = [1; 1];
		let _ = self.stop_write.write(buf.as_slice());
		let _ = self
			.join_handle
			.get_mut()
			.ok()
			.and_then(|handle| handle.take())
			.and_then(|handle| handle.join().ok())
			.expect("Couldn't join the event loop thread at drop");
	}
}

use super::eventloop::EventLoop;
use super::scenegraph::Scenegraph;
use crate::nodes::field;
use crate::nodes::spatial;
use libstardustxr::messenger::Messenger;
use mio::net::UnixStream;
use std::rc::Rc;
use std::sync::{Arc, Weak};

pub struct Client<'a> {
	event_loop: Weak<EventLoop>,
	messenger: Messenger<'a>,
	scenegraph: Scenegraph<'a>,
}

impl<'a> Client<'a> {
	pub fn from_connection(connection: UnixStream, event_loop_ref: &Arc<EventLoop>) -> Rc<Self> {
		let client = Rc::new(Client {
			event_loop: Arc::downgrade(event_loop_ref),
			messenger: Messenger::new(connection),
			scenegraph: Default::default(),
		});
		client.scenegraph.set_client(&client);
		spatial::create_interface(client.clone());
		field::create_interface(client.clone());
		client
	}
	pub fn dispatch(&self) -> Result<(), std::io::Error> {
		self.messenger.dispatch(&self.scenegraph)
	}

	// pub fn get_messenger(&self) -> &Messenger<'a> {
	// 	&self.messenger
	// }
	pub fn get_scenegraph(&self) -> &Scenegraph<'a> {
		&self.scenegraph
	}
}

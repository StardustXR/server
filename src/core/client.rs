use super::scenegraph::Scenegraph;
use crate::nodes::spatial;
use libstardustxr::messenger::Messenger;
use mio::net::UnixStream;
use rccell::{RcCell, WeakCell};

pub struct Client<'a> {
	pub messenger: Messenger<'a>,
	scenegraph: Scenegraph<'a>,
}

impl<'a> Client<'a> {
	pub fn from_connection(connection: UnixStream) -> RcCell<Self> {
		let client = RcCell::new(Client {
			scenegraph: Default::default(),
			messenger: Messenger::new(connection),
		});
		client.borrow_mut().scenegraph.set_client(client.clone());
		spatial::create_interface(client.clone());
		client
	}
	pub fn dispatch(&self) -> Result<(), std::io::Error> {
		self.messenger.dispatch(&self.scenegraph)
	}

	pub fn get_messenger(&self) -> &Messenger<'a> {
		&self.messenger
	}
	pub fn get_scenegraph(&self) -> &Scenegraph<'a> {
		&self.scenegraph
	}
	pub fn get_scenegraph_mut(&mut self) -> &mut Scenegraph<'a> {
		&mut self.scenegraph
	}
}

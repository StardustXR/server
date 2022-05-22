use super::scenegraph::Scenegraph;
use libstardustxr::messenger::Messenger;
use mio::net::UnixStream;
use std::rc::{Rc, Weak};

pub struct Client<'a> {
	messenger: Rc<Messenger<'a>>,
	pub scenegraph: Option<Scenegraph<'a>>,
}

impl<'a> Client<'a> {
	pub fn from_connection(connection: UnixStream) -> Self {
		let mut client = Client {
			scenegraph: None,
			messenger: Rc::new(Messenger::new(connection)),
		};
		client.scenegraph = Some(Scenegraph::new(&mut client));
		client
	}
	pub fn dispatch(&self) -> Result<(), std::io::Error> {
		self.messenger.dispatch(self.scenegraph.as_ref().unwrap())
	}

	pub fn get_weak_messenger(&self) -> Weak<Messenger<'a>> {
		Rc::downgrade(&self.messenger)
	}
}

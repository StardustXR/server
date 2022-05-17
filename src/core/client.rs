use super::scenegraph::Scenegraph;
use crate::nodes::core::{Node, NodeRef};
use libstardustxr::messenger::Messenger;
use mio::net::UnixStream;
use std::rc::{Rc, Weak};

pub struct Client<'a> {
	pub messenger: Rc<Messenger<'a>>,
	pub scenegraph: Scenegraph<'a>,
}

impl<'a> Client<'a> {
	pub fn from_connection(connection: UnixStream) -> Self {
		Client {
			scenegraph: Default::default(),
			messenger: Rc::new(Messenger::new(connection)),
		}
	}
	pub fn dispatch(&self) -> Result<(), std::io::Error> {
		self.messenger.dispatch(&self.scenegraph)
	}

	pub fn get_weak_messenger(&self) -> Weak<Messenger<'a>> {
		Rc::downgrade(&self.messenger)
	}
}

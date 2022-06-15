use super::scenegraph::Scenegraph;
use crate::nodes::field;
use crate::nodes::spatial;
use libstardustxr::messenger::Messenger;
use mio::net::UnixStream;
use std::sync::Arc;

pub struct Client {
	pub messenger: Messenger,
	pub scenegraph: Scenegraph,
}

impl Client {
	pub fn from_connection(connection: UnixStream) -> Arc<Self> {
		let client = Arc::new(Client {
			messenger: Messenger::new(connection),
			scenegraph: Default::default(),
		});
		let _ = client.scenegraph.client.set(Arc::downgrade(&client));
		spatial::create_interface(&client);
		field::create_interface(&client);
		client
	}
	pub fn dispatch(&self) -> Result<(), std::io::Error> {
		self.messenger.dispatch(&self.scenegraph)
	}
}

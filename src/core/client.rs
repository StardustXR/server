use super::scenegraph::Scenegraph;
use crate::nodes::data;
use crate::nodes::field;
use crate::nodes::input;
use crate::nodes::item;
use crate::nodes::root;
use crate::nodes::spatial;
use lazy_static::lazy_static;
use libstardustxr::messenger::Messenger;
use mio::net::UnixStream;
use std::sync::Arc;

lazy_static! {
	pub static ref INTERNAL_CLIENT: Arc<Client> = Default::default();
}

#[derive(Default)]
pub struct Client {
	pub messenger: Option<Messenger>,
	pub scenegraph: Scenegraph,
}

impl Client {
	pub fn from_connection(connection: UnixStream) -> Arc<Self> {
		let client = Arc::new(Client {
			messenger: Some(Messenger::new(connection)),
			scenegraph: Default::default(),
		});
		let _ = client.scenegraph.client.set(Arc::downgrade(&client));
		root::create_root(&client);
		spatial::create_interface(&client);
		field::create_interface(&client);
		data::create_interface(&client);
		item::create_interface(&client);
		input::create_interface(&client);
		client
	}
	pub fn dispatch(&self) -> Result<(), std::io::Error> {
		if let Some(messenger) = &self.messenger {
			messenger.dispatch(&self.scenegraph)
		} else {
			Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
		}
	}
}

use super::scenegraph::Scenegraph;
use crate::nodes::data;
use crate::nodes::field;
use crate::nodes::input;
use crate::nodes::item;
use crate::nodes::root::Root;
use crate::nodes::spatial;
use lazy_static::lazy_static;
use libstardustxr::messenger::Messenger;
use mio::net::UnixStream;
use once_cell::sync::OnceCell;
use std::sync::Arc;

lazy_static! {
	pub static ref INTERNAL_CLIENT: Arc<Client> = Client::new_local();
}

pub struct Client {
	pub messenger: Option<Messenger>,
	pub scenegraph: Scenegraph,
	pub root: OnceCell<Arc<Root>>,
}
impl Client {
	pub fn new_local() -> Arc<Self> {
		Arc::new(Client {
			messenger: None,
			scenegraph: Default::default(),
			root: OnceCell::new(),
		})
	}
	pub fn from_connection(connection: UnixStream) -> Arc<Self> {
		println!("New client connected");
		let client = Arc::new(Client {
			messenger: Some(Messenger::new(connection)),
			scenegraph: Default::default(),
			root: OnceCell::new(),
		});
		let _ = client.scenegraph.client.set(Arc::downgrade(&client));
		let _ = client.root.set(Root::create(&client));
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
impl Drop for Client {
	fn drop(&mut self) {
		println!("Client disconnected");
	}
}

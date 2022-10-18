use super::{eventloop::EventLoop, scenegraph::Scenegraph};
use crate::{
	core::registry::Registry,
	nodes::{data, drawable, fields, hmd, input, items, root::Root, spatial, startup, Node},
};
use anyhow::{anyhow, Result};
use lazy_static::lazy_static;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use stardust_xr::messenger::Messenger;
use std::{
	path::PathBuf,
	sync::{Arc, Weak},
};
use tokio::{net::UnixStream, sync::Notify, task::JoinHandle};

lazy_static! {
	pub static ref CLIENTS: Registry<Client> = Registry::new();
	pub static ref INTERNAL_CLIENT: Arc<Client> = CLIENTS.add(Client {
		event_loop: Weak::new(),
		index: 0,

		stop_notifier: Default::default(),
		join_handle: OnceCell::new(),

		messenger: None,
		scenegraph: Default::default(),
		root: OnceCell::new(),
		base_resource_prefixes: Default::default(),
	});
}

pub struct Client {
	event_loop: Weak<EventLoop>,
	index: usize,
	stop_notifier: Arc<Notify>,
	join_handle: OnceCell<JoinHandle<Result<()>>>,

	pub messenger: Option<Messenger>,
	pub scenegraph: Scenegraph,
	pub root: OnceCell<Arc<Root>>,
	pub base_resource_prefixes: Mutex<Vec<PathBuf>>,
}
impl Client {
	pub fn from_connection(
		index: usize,
		event_loop: &Arc<EventLoop>,
		connection: UnixStream,
	) -> Arc<Self> {
		println!("New client connected");
		let client = CLIENTS.add(Client {
			event_loop: Arc::downgrade(event_loop),
			index,
			stop_notifier: Default::default(),
			join_handle: OnceCell::new(),

			messenger: Some(Messenger::new(
				tokio::runtime::Handle::current(),
				connection,
			)),
			scenegraph: Default::default(),
			root: OnceCell::new(),
			base_resource_prefixes: Default::default(),
		});
		let _ = client.scenegraph.client.set(Arc::downgrade(&client));
		let _ = client.root.set(Root::create(&client));
		hmd::make_alias(&client);
		spatial::create_interface(&client);
		fields::create_interface(&client);
		drawable::create_interface(&client);
		data::create_interface(&client);
		items::create_interface(&client);
		input::create_interface(&client);
		startup::create_interface(&client);

		let _ = client.join_handle.set(tokio::spawn({
			let client = client.clone();
			async move {
				let dispatch_loop = async {
					loop {
						client.dispatch().await?
					}
				};
				let flush_loop = async {
					loop {
						client.flush().await?
					}
				};

				let result = tokio::select! {
					_ = client.stop_notifier.notified() => Ok(()),
					e = dispatch_loop => e,
					e = flush_loop => e,
				};
				client.disconnect().await;
				result
			}
		}));
		client
	}

	#[inline]
	pub fn get_node(&self, name: &'static str, path: &str) -> Result<Arc<Node>> {
		self.scenegraph
			.get_node(path)
			.ok_or_else(|| anyhow!("{} not found", name))
	}

	pub async fn dispatch(&self) -> Result<(), std::io::Error> {
		match &self.messenger {
			Some(messenger) => messenger.dispatch(&self.scenegraph).await,
			None => Err(std::io::Error::from(std::io::ErrorKind::Unsupported)),
		}
	}

	pub async fn flush(&self) -> Result<(), std::io::Error> {
		match &self.messenger {
			Some(messenger) => messenger.flush().await,
			None => Err(std::io::Error::from(std::io::ErrorKind::Unsupported)),
		}
	}

	pub async fn disconnect(&self) {
		self.stop_notifier.notify_one();
		if let Some(event_loop) = self.event_loop.upgrade() {
			event_loop.clients.lock().await.remove(self.index);
		}
	}
}
impl Drop for Client {
	fn drop(&mut self) {
		self.stop_notifier.notify_one();
		CLIENTS.remove(self);
		println!("Client disconnected");
	}
}

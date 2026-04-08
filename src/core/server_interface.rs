use std::path::PathBuf;

use gluon_wire::drop_tracking::DropNotifier;
use stardust_xr_protocol::{
	client::ClientState,
	server::{Server, ServerInterfaceHandler},
};
use tokio::sync::RwLock;

use crate::{core::client::ConnectedClient, impl_transaction_handler};

#[derive(Debug, Default)]
pub struct ServerInterface {
	drop_notifs: RwLock<Vec<DropNotifier>>,
}
impl_transaction_handler!(ServerInterface);
impl ServerInterfaceHandler for ServerInterface {
	async fn connect(
		&self,
		ctx: gluon_wire::GluonCtx,
		client: stardust_xr_protocol::client::Client,
		resource_prefixes: Vec<String>,
	) -> (Server, ClientState) {
		// TODO: forward errors
		let (obj, state) = ConnectedClient::from_connection(
			client,
			ctx.sender_pid,
			resource_prefixes.into_iter().map(PathBuf::from).collect(),
		)
		.await
		.unwrap();

		(Server::from_handler(&obj), state)
	}

	// TODO: this causes a mem leak on connect!
	async fn drop_notification_requested(&self, notifier: DropNotifier) {
		self.drop_notifs.write().await.push(notifier);
	}
}

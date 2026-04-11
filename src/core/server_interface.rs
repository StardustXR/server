use std::path::PathBuf;

use stardust_xr_protocol::{
	client::ClientState,
	server::{Server, ServerInterfaceHandler},
};

use crate::{core::client::ConnectedClient, exposed_interface};

exposed_interface!(ServerInterface, "stardust-server");
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
		.unwrap();

		(Server::from_handler(&obj), state)
	}
}

use std::path::PathBuf;

use stardust_xr_protocol::{
	server::{Server, ServerInterfaceHandler},
	spatial::SpatialRef,
};

use crate::{
	core::client::{ConnectedClient, state},
	exposed_interface,
};

exposed_interface!(ServerInterface, "stardust-server");
impl ServerInterfaceHandler for ServerInterface {
	async fn connect(
		&self,
		_ctx: gluon_wire::GluonCtx,
		client: stardust_xr_protocol::client::Client,
		startup_token: Option<String>,
		resource_prefixes: Vec<String>,
	) -> (Server, SpatialRef) {
		// TODO: forward errors
		let (obj, state) = ConnectedClient::from_connection(
			client,
			startup_token,
			resource_prefixes.into_iter().map(PathBuf::from).collect(),
		)
		.unwrap();

		(Server::from_handler(&obj), state)
	}

	async fn startup_spatial(
		&self,
		_ctx: gluon_wire::GluonCtx,
		startup_token: String,
	) -> Option<SpatialRef> {
		Some(state(&startup_token)?.apply())
	}
}

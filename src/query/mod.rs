use std::{
	collections::HashMap,
	sync::{Arc, LazyLock, Weak},
};

use bevy::prelude::Deref;
use binderbinder::binder_object::{BinderObject, BinderObjectOrRef, BinderObjectRef};
use gluon_wire::{GluonCtx, Handler};
use stardust_xr_protocol::query::{
	QueryInterfaceHandler, QueryableError, QueryableInterfaceGuard, QueryableInterfaceGuardHandler,
	QueryableObject, QueryableObjectHandler, QueryableObjectRef, QueryableObjectRefHandler,
};
use stardust_xr_server_foundation::{deduped_string::DedupedStr, registry::Registry};
use tokio::sync::RwLock;
use tracing::info;

use crate::{
	PION, interface,
	nodes::{ProxyExt, fields::FieldRef, spatial::SpatialRef},
	query::spatial_query::Query,
};

pub mod spatial_query;

static QUERY_STATE: LazyLock<State> = LazyLock::new(State::default);
#[derive(Default)]
struct State {
	interface_to_queryable: RwLock<HashMap<Arc<DedupedStr>, Registry<Queryable>>>,
	queries: Registry<Query>,
}
#[derive(Debug, Handler)]
struct QueryableRef;
impl QueryableObjectRefHandler for QueryableRef {}

#[derive(Debug, Deref, Handler)]
struct QueryableMut(Arc<Queryable>);
#[derive(Debug)]
struct Queryable {
	queryable_ref: BinderObject<QueryableRef>,
	spatial: BinderObjectRef<SpatialRef>,
	field: BinderObjectRef<FieldRef>,
	interfaces: RwLock<Registry<QueryableInterface>>,
}
#[derive(Debug)]
struct QueryableInterface {
	interface_id: Arc<DedupedStr>,
	interface_ref: BinderObjectOrRef,
}
#[derive(Debug, Handler)]
struct InterfaceGuard(Option<Arc<QueryableInterface>>, Weak<Queryable>);
impl QueryableInterfaceGuardHandler for InterfaceGuard {}
impl Drop for InterfaceGuard {
	fn drop(&mut self) {
		info!("dropping interface");
		drop(self.0.take());
		if let Some(queryable) = self.1.upgrade() {
			tokio::spawn(async move { queryable.notify_interface_changes().await });
		}
	}
}
impl QueryableObjectHandler for QueryableMut {
	async fn queryable_ref(&self, _ctx: gluon_wire::GluonCtx) -> QueryableObjectRef {
		QueryableObjectRef::from_handler(&self.queryable_ref)
	}

	async fn add_interface(
		&self,
		_ctx: gluon_wire::GluonCtx,
		interface: binderbinder::binder_object::BinderObjectOrRef,
		interface_id: String,
	) -> QueryableInterfaceGuard {
		let interface = self.interfaces.write().await.add(QueryableInterface {
			interface_id: DedupedStr::get(interface_id).await,
			interface_ref: interface,
		});
		self.notify_interface_changes().await;
		let guard = PION.register_object(InterfaceGuard(Some(interface), Arc::downgrade(&self.0)));
		QueryableInterfaceGuard::from_handler(&guard.to_service())
	}
}
impl Queryable {
	async fn notify_interface_changes(self: &Arc<Queryable>) {
		let queries = QUERY_STATE.queries.get_valid_contents();
		for query in queries {
			query.update_interfaces(self).await;
		}
	}
}
impl Drop for Queryable {
	fn drop(&mut self) {
		QUERY_STATE
			.queries
			.get_valid_contents()
			.into_iter()
			.for_each(|q| q.queryable_destroyed(self));
	}
}

interface!(QueryInterface);
impl QueryInterfaceHandler for QueryInterface {
	async fn register_queryable(
		&self,
		_ctx: GluonCtx,
		spatial: stardust_xr_protocol::spatial::SpatialRef,
		field: stardust_xr_protocol::field::FieldRef,
	) -> Result<QueryableObject, QueryableError> {
		let spatial = spatial.owned().ok_or(QueryableError::InvalidField)?;
		let field = field.owned().ok_or(QueryableError::InvalidField)?;
		let queryable_ref = PION.register_object(QueryableRef);
		let queryable = Arc::new(Queryable {
			field,
			spatial,
			interfaces: RwLock::default(),
			queryable_ref,
		});
		let obj = PION.register_object(QueryableMut(queryable));
		Ok(QueryableObject::from_handler(&obj.to_service()))
	}
}

#[derive(Debug)]
struct InterfaceQuery {
	id: Arc<DedupedStr>,
	optional: bool,
}

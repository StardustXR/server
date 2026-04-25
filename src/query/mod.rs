use std::{
	collections::HashMap,
	sync::{Arc, LazyLock, Weak},
};

use bevy::prelude::Deref;
use binderbinder::binder_object::{BinderObject, BinderObjectOrRef};
use gluon_wire::{GluonCtx, impl_transaction_handler};
use stardust_xr_protocol::query::{
	QueryInterfaceHandler, QueryableError, QueryableInterfaceGuard, QueryableInterfaceGuardHandler,
	QueryableObject, QueryableObjectHandler, QueryableObjectRef, QueryableObjectRefHandler,
};
use stardust_xr_server_foundation::{deduped_string::DedupedStr, registry::Registry};
use tokio::sync::RwLock;

use crate::{
	PION, interface,
	nodes::{ProxyExt, fields::FieldRef},
	query::spatial_query::Query,
};
use stardust_xr_protocol::field::FieldRef as FieldRefProxy;

pub mod spatial_query;

static QUERY_STATE: LazyLock<State> = LazyLock::new(State::default);
#[derive(Default)]
struct State {
	interface_to_queryable: RwLock<HashMap<Arc<DedupedStr>, Registry<Queryable>>>,
	queries: Registry<Query>,
}
#[derive(Debug)]
struct QueryableRef;
impl QueryableObjectRefHandler for QueryableRef {}

#[derive(Debug, Deref)]
struct QueryableMut(Arc<Queryable>);
#[derive(Debug)]
struct Queryable {
	queryable_ref: BinderObject<QueryableRef>,
	field: Arc<FieldRef>,
	field_proxy: FieldRefProxy,
	path: Arc<DedupedStr>,
	interfaces: RwLock<Registry<QueryableInterface>>,
}
#[derive(Debug)]
struct QueryableInterface {
	interface_id: Arc<DedupedStr>,
	interface_ref: BinderObjectOrRef,
}
#[derive(Debug)]
struct InterfaceGuard(Option<Arc<QueryableInterface>>, Weak<Queryable>);
impl QueryableInterfaceGuardHandler for InterfaceGuard {}
impl Drop for InterfaceGuard {
	fn drop(&mut self) {
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
impl_transaction_handler!(QueryableMut);
impl_transaction_handler!(QueryableRef);
impl_transaction_handler!(InterfaceGuard);

interface!(QueryInterface);
impl QueryInterfaceHandler for QueryInterface {
	async fn register_queryable(
		&self,
		_ctx: GluonCtx,
		field: stardust_xr_protocol::field::FieldRef,
		path: String,
	) -> Result<QueryableObject, QueryableError> {
		let field_proxy = field.clone();
		let field = field.owned().ok_or(QueryableError::InvalidField)?;
		// TODO: make sure path is valid
		let path = DedupedStr::get(path).await;
		let queryable_ref = PION.register_object(QueryableRef);
		let queryable = Arc::new(Queryable {
			field,
			field_proxy,
			path,
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

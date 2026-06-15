use crate::{
	PION, interface,
	nodes::{ProxyExt, fields::FieldObject, spatial::SpatialObject},
	query::spatial_query::Query,
};
use bevy::prelude::Deref;
use gluon::{Handler, Object, ObjectOrRef, ObjectRef};
use stardust_xr_protocol::query::{
	QueryInterfaceHandler, QueryableError, QueryableInterfaceGuard, QueryableInterfaceGuardHandler,
	QueryableObject, QueryableObjectHandler, QueryableObjectRef, QueryableObjectRefHandler,
};
use stardust_xr_server_foundation::{deduped_string::DedupedStr, registry::Registry};
use std::{
	collections::HashMap,
	sync::{
		Arc, LazyLock, Weak,
		atomic::{AtomicU64, Ordering},
	},
};
use tokio::sync::RwLock;
use tracing::info;

pub mod spatial_query;
#[cfg(test)]
mod tests;

static QUERY_STATE: LazyLock<State> = LazyLock::new(State::default);
/// Hands out a stable, never-reused id to each queryable so query bookkeeping can
/// key on identity without relying on (recyclable) pointer addresses.
static NEXT_QUERYABLE_ID: AtomicU64 = AtomicU64::new(0);
#[derive(Default)]
struct State {
	interface_to_queryable: RwLock<HashMap<Arc<DedupedStr>, Registry<Queryable>>>,
	/// Every live queryable, so a freshly-created query can discover the ones that
	/// already exist (self-inserts on registration, drops out via `Drop`).
	all_queryables: Registry<Queryable>,
	queries: Registry<Query>,
}
#[derive(Debug, Handler)]
struct QueryableRef;
impl QueryableObjectRefHandler for QueryableRef {}

#[derive(Debug, Deref, Handler)]
struct QueryableMut(Arc<Queryable>);
#[derive(Debug)]
struct Queryable {
	id: u64,
	queryable_ref: Object<QueryableRef>,
	spatial: ObjectRef<SpatialObject>,
	field: ObjectRef<FieldObject>,
	interfaces: RwLock<Registry<QueryableInterface>>,
}
#[derive(Debug)]
struct QueryableInterface {
	interface_id: Arc<DedupedStr>,
	interface_ref: ObjectOrRef,
}
#[derive(Debug, Handler)]
struct InterfaceGuard(Option<Arc<QueryableInterface>>, Weak<Queryable>);
impl QueryableInterfaceGuardHandler for InterfaceGuard {}
impl Drop for InterfaceGuard {
	fn drop(&mut self) {
		info!("Dropping interface");
		drop(self.0.take());
		if let Some(queryable) = self.1.upgrade() {
			tokio::spawn(async move { queryable.notify_interface_changes().await });
		}
	}
}
impl QueryableObjectHandler for QueryableMut {
	async fn queryable_ref(&self, _ctx: gluon::Context) -> QueryableObjectRef {
		QueryableObjectRef::from_handler(&self.queryable_ref)
	}

	async fn add_interface(
		&self,
		_ctx: gluon::Context,
		interface: ObjectOrRef,
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
			query.update_hit_queryable(self).await;
		}
	}
}
impl Drop for Queryable {
	fn drop(&mut self) {
		QUERY_STATE.all_queryables.remove(self);
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
		_ctx: gluon::Context,
		spatial: stardust_xr_protocol::spatial::Spatial,
		field: stardust_xr_protocol::field::Field,
	) -> Result<QueryableObject, QueryableError> {
		info!(?spatial, ?field, "Registered queryable");
		let spatial = spatial.owned().ok_or(QueryableError::NotOwnedSpatial)?;
		let field = field.owned().ok_or(QueryableError::NotOwnedField)?;
		let queryable_ref = PION.register_object(QueryableRef);
		let queryable = Arc::new(Queryable {
			id: NEXT_QUERYABLE_ID.fetch_add(1, Ordering::Relaxed),
			field,
			spatial,
			interfaces: RwLock::default(),
			queryable_ref,
		});
		QUERY_STATE.all_queryables.add_raw(&queryable);
		let obj = PION.register_object(QueryableMut(queryable));
		Ok(QueryableObject::from_handler(&obj.to_service()))
	}
}

#[derive(Debug)]
struct InterfaceQuery {
	id: Arc<DedupedStr>,
	optional: bool,
}

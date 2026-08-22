use crate::{
	interface,
	nodes::{ProxyExt, fields::FieldObject, spatial::SpatialObject},
	query::spatial_query::AnyQuery,
};
use bevy::prelude::Deref;
use gluon::{Handler, LocalRef, Ref, RefExt};
use stardust_xr_protocol::{
	field::Field,
	query::{
		QueryInterfaceHandler, QueryableError, QueryableId,
		QueryableInterface as QueryableInterfaceProxy, QueryableInterfaceHandler, QueryableObject,
		QueryableObjectHandler,
	},
	spatial::Spatial,
};
use stardust_xr_server_foundation::{deduped_string::DedupedStr, registry::Registry};
use std::{
	future::ready,
	sync::{
		Arc, LazyLock, Weak,
		atomic::{AtomicU64, Ordering},
	},
};
use tokio::sync::RwLock;
use tracing::debug;

pub mod spatial_query;
#[cfg(test)]
mod tests;

static QUERY_STATE: LazyLock<State> = LazyLock::new(State::default);
/// Hands out a stable, never-reused id to each queryable so query bookkeeping can
/// key on identity without relying on (recyclable) pointer addresses.
static NEXT_QUERYABLE_ID: AtomicU64 = AtomicU64::new(0);
#[derive(Default)]
struct State {
	/// Every live queryable, so a freshly-created query can discover the ones that
	/// already exist (self-inserts on registration, drops out via `Drop`).
	all_queryables: Registry<Queryable>,
	queries: Registry<dyn AnyQuery>,
}

#[derive(Debug, Deref, Handler)]
struct QueryableMut(Arc<Queryable>);
#[derive(Debug)]
struct Queryable {
	id: QueryableId,
	spatial: LocalRef<Spatial, SpatialObject>,
	field: LocalRef<Field, FieldObject>,
	interfaces: RwLock<Registry<QueryableInterface>>,
	/// Serializes interface re-syncs for this queryable (see
	/// `Query::update_interfaces_impl`). Interface *removal* is an `Arc` drop, not a
	/// write to `interfaces`, so the `RwLock` alone cannot order a snapshot against a
	/// concurrent change — without this, a stale snapshot could be applied last and
	/// the tracked interface set would disagree with reality until the next change.
	update_lock: tokio::sync::Mutex<()>,
}
#[derive(Debug)]
struct QueryableInterface {
	interface_id: Arc<DedupedStr>,
	interface_ref: gluon::Ref,
}
#[derive(Debug, Handler)]
struct InterfaceGuard(Option<Arc<QueryableInterface>>, Weak<Queryable>);
impl QueryableInterfaceHandler for InterfaceGuard {}
impl Drop for InterfaceGuard {
	fn drop(&mut self) {
		let i = self.0.take().unwrap();

		debug!(
			interface = i.interface_id.get_string(),
			"Dropping queryable interface"
		);
		if let Some(queryable) = self.1.upgrade() {
			tokio::spawn(async move { queryable.notify_interface_changes().await });
		}
	}
}
impl QueryableObjectHandler for QueryableMut {
	fn id(&self, _ctx: gluon::Context) -> impl Future<Output = QueryableId> + Send + Sync {
		ready(self.id)
	}

	async fn add_interface(
		&self,
		_ctx: gluon::Context,
		interface: Ref,
		interface_id: String,
	) -> Result<QueryableInterfaceProxy, QueryableError> {
		debug!(?self, interface = interface_id, "Registered interface");
		// TODO: detect duplicate interfaces?
		let interface = self.interfaces.write().await.add(QueryableInterface {
			interface_id: DedupedStr::get(interface_id).await,
			interface_ref: interface,
		});
		self.notify_interface_changes().await;
		let interface = QueryableInterfaceProxy::new_service(InterfaceGuard(
			Some(interface),
			Arc::downgrade(&self.0),
		))
		// TODO: somehow remove this unwrap?
		.unwrap()
		.into_proxy();
		Ok(interface)
	}
}
impl Queryable {
	async fn notify_interface_changes(self: &Arc<Queryable>) {
		let queries = QUERY_STATE.queries.get_valid_contents();
		for query in queries {
			query.update_interfaces(self.clone()).await;
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
		debug!(?spatial, ?field, "Registered queryable");
		let spatial = spatial.owned_ref().ok_or(QueryableError::NotOwnedSpatial)?;
		let field = field.owned_ref().ok_or(QueryableError::NotOwnedField)?;
		let queryable = Arc::new(Queryable {
			id: QueryableId {
				id: NEXT_QUERYABLE_ID.fetch_add(1, Ordering::Relaxed),
			},
			field,
			spatial,
			interfaces: RwLock::default(),
			update_lock: tokio::sync::Mutex::new(()),
		});
		QUERY_STATE.all_queryables.add_raw(&queryable);
		let obj = QueryableObject::new_service(QueryableMut(queryable))
			// TODO: remove that unwrap
			.unwrap()
			.into_proxy();
		Ok(obj)
	}
}

#[derive(Debug)]
struct InterfaceQuery {
	id: Arc<DedupedStr>,
	optional: bool,
}

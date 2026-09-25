use crate::{
	interface,
	nodes::{ProxyExt, fields::FieldObject, spatial::SpatialObject},
	query::spatial_query::AnyQuery,
};
use bevy::prelude::Deref;
use gluon_ipc::{Handler, LocalRef, Ref, RefExt};
use parking_lot::Mutex;
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
	collections::HashMap,
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
	/// every queryable that has ever had each interface, so a new query only visits the
	/// ones that could match. dead ones fall out on their own, and ones that lost the
	/// interface since just get checked and skipped
	by_interface: Mutex<HashMap<Arc<DedupedStr>, Arc<Registry<Queryable>>>>,
	queries: Registry<dyn AnyQuery>,
}
fn index_interface(queryable: &Arc<Queryable>, interface_id: &Arc<DedupedStr>) {
	let registry = QUERY_STATE
		.by_interface
		.lock()
		.entry(interface_id.clone())
		.or_default()
		.clone();
	registry.add_raw(queryable);
}
fn queryables_with(interface_id: &Arc<DedupedStr>) -> Vec<Arc<Queryable>> {
	let registry = QUERY_STATE.by_interface.lock().get(interface_id).cloned();
	registry.map_or_else(Vec::new, |r| r.get_valid_contents())
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
	interface_ref: gluon_ipc::Ref,
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
	fn id(&self, _ctx: gluon_ipc::Context) -> impl Future<Output = QueryableId> + Send + Sync {
		ready(self.id)
	}

	async fn add_interface(
		&self,
		_ctx: gluon_ipc::Context,
		interface: Ref,
		interface_id: String,
	) -> Result<QueryableInterfaceProxy, QueryableError> {
		debug!(?self, interface = interface_id, "Registered interface");
		// TODO: detect duplicate interfaces?
		let interface_id = DedupedStr::get(interface_id).await;
		// indexed before anyone is told, so a new query either finds it or hears about it
		index_interface(&self.0, &interface_id);
		let interface = self.interfaces.write().await.add(QueryableInterface {
			interface_id,
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
	fn new(
		spatial: LocalRef<Spatial, SpatialObject>,
		field: LocalRef<Field, FieldObject>,
	) -> Arc<Queryable> {
		let queryable = Arc::new(Queryable {
			id: QueryableId {
				id: NEXT_QUERYABLE_ID.fetch_add(1, Ordering::Relaxed),
			},
			field,
			spatial,
			interfaces: RwLock::default(),
			update_lock: tokio::sync::Mutex::new(()),
		});
		queryable
	}
	async fn notify_interface_changes(self: &Arc<Queryable>) {
		let queries = QUERY_STATE.queries.get_valid_contents();
		for query in queries {
			query.update_interfaces(self.clone()).await;
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

/// a queryable the server owns itself, listed for as long as this is held
#[derive(Debug)]
pub struct ServerQueryable {
	_queryable: Arc<Queryable>,
	_interfaces: Vec<Arc<QueryableInterface>>,
}
impl ServerQueryable {
	pub async fn new(
		spatial: LocalRef<Spatial, SpatialObject>,
		field: LocalRef<Field, FieldObject>,
		interfaces: impl IntoIterator<Item = (&str, Ref)>,
	) -> Self {
		let queryable = Queryable::new(spatial, field);
		let mut held = Vec::new();
		for (id, interface_ref) in interfaces {
			let interface_id = DedupedStr::get(id.to_string()).await;
			index_interface(&queryable, &interface_id);
			held.push(queryable.interfaces.write().await.add(QueryableInterface {
				interface_id,
				interface_ref,
			}));
		}
		queryable.notify_interface_changes().await;
		ServerQueryable {
			_queryable: queryable,
			_interfaces: held,
		}
	}
}

interface!(QueryInterface);
impl QueryInterfaceHandler for QueryInterface {
	async fn register_queryable(
		&self,
		_ctx: gluon_ipc::Context,
		spatial: stardust_xr_protocol::spatial::Spatial,
		field: stardust_xr_protocol::field::Field,
	) -> Result<QueryableObject, QueryableError> {
		debug!(?spatial, ?field, "Registered queryable");
		let spatial = spatial.owned_ref().ok_or(QueryableError::NotOwnedSpatial)?;
		let field = field.owned_ref().ok_or(QueryableError::NotOwnedField)?;
		let obj = QueryableObject::new_service(QueryableMut(Queryable::new(spatial, field)))
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

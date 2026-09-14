use crate::nodes::{ProxyExt as _, fields::Field, spatial::SpatialRef};
use gluon_ipc::{Context, Handler, RefExt};
use stardust_xr_protocol::{
	field::{FieldRef as FieldRefProxy, FieldSample, RayMarchResult},
	query::{QueriedInterface, QueryableId},
	spatial::SpatialRef as SpatialRefProxy,
	spatial_query::{BeamQueryHandlerHandler, PointsQueryHandlerHandler},
	suis::InputHandler,
};
use std::{
	collections::{HashMap, HashSet},
	fmt::{Debug, Formatter},
	sync::Arc,
};
use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

/// An input handler the query found, with everything needed to send it input.
pub struct CachedHandler<V: Send + Sync + 'static> {
	pub handler: InputHandler,
	/// None while the get_spatial round trip is in flight, filtered from dispatch until populated
	pub spatial: Option<Arc<SpatialRef>>,
	pub field: Arc<Field>,
	pub value: V,
	/// the handler left the query but is retained because it holds a capture
	pub left_query: bool,
}
impl<V: Send + Sync + 'static> Debug for CachedHandler<V> {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("CachedHandler")
			.field("spatial", &self.spatial.is_some())
			.field("left_query", &self.left_query)
			.finish_non_exhaustive()
	}
}

/// The input handlers a spatial query is currently turning up, shared between the query
/// adapter and the [`super::InputMethod`] that dispatches to them.
pub struct QueryCache<V: Send + Sync + 'static> {
	handlers: Arc<RwLock<HashMap<QueryableId, CachedHandler<V>>>>,
	capture_requests: Arc<RwLock<HashSet<InputHandler>>>,
}
impl<V: Send + Sync + 'static> Clone for QueryCache<V> {
	fn clone(&self) -> Self {
		Self {
			handlers: self.handlers.clone(),
			capture_requests: self.capture_requests.clone(),
		}
	}
}
impl<V: Send + Sync + 'static> Default for QueryCache<V> {
	fn default() -> Self {
		Self {
			handlers: Arc::default(),
			capture_requests: Arc::default(),
		}
	}
}
impl<V: Send + Sync + 'static> Debug for QueryCache<V> {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("QueryCache").finish_non_exhaustive()
	}
}

impl<V: Send + Sync + 'static> QueryCache<V> {
	pub async fn handlers(&self) -> RwLockReadGuard<'_, HashMap<QueryableId, CachedHandler<V>>> {
		self.handlers.read().await
	}
	pub fn handlers_blocking(&self) -> RwLockReadGuard<'_, HashMap<QueryableId, CachedHandler<V>>> {
		self.handlers.blocking_read()
	}
	pub(super) async fn handlers_mut(
		&self,
	) -> RwLockWriteGuard<'_, HashMap<QueryableId, CachedHandler<V>>> {
		self.handlers.write().await
	}
	pub async fn capture_requests(&self) -> RwLockReadGuard<'_, HashSet<InputHandler>> {
		self.capture_requests.read().await
	}
	pub fn capture_requests_blocking(&self) -> RwLockReadGuard<'_, HashSet<InputHandler>> {
		self.capture_requests.blocking_read()
	}
	pub(super) async fn capture_requests_mut(&self) -> RwLockWriteGuard<'_, HashSet<InputHandler>> {
		self.capture_requests.write().await
	}

	async fn on_entered(
		&self,
		obj: QueryableId,
		field: FieldRefProxy,
		interfaces: Vec<QueriedInterface>,
		value: V,
	) {
		let Some(interface) = interfaces
			.iter()
			.find(|i| i.interface_id == InputHandler::QUERY_INTERFACE)
		else {
			return;
		};
		let Some(field) = field.owned() else { return };
		let handler = InputHandler::from_ref(interface.interface.clone());

		{
			let mut handlers = self.handlers.write().await;
			if let Some(entry) = handlers.get_mut(&obj) {
				// Re-entered the query, which a beam does every time it flaps across a field
				// boundary. Keep the existing spatial: nulling it would momentarily drop a
				// retained handler out of dispatch and emit a spurious input_left.
				entry.field = field.data.clone();
				entry.value = value;
				entry.left_query = false;
				return;
			}

			// Insert a sentinel before the get_spatial round trip so on_left can still find
			// and remove this entry while that call is in flight.
			handlers.insert(
				obj,
				CachedHandler {
					handler: handler.clone(),
					spatial: None,
					field: field.data.clone(),
					value,
					left_query: false,
				},
			);
		}

		let spatial = handler.get_spatial().await.ok().and_then(|s| s.owned());
		let Some(spatial) = spatial else {
			self.handlers.write().await.remove(&obj);
			return;
		};

		if let Some(entry) = self.handlers.write().await.get_mut(&obj)
			&& entry.spatial.is_none()
		{
			entry.spatial = Some(spatial);
		}
	}

	async fn on_value_changed(&self, obj: &QueryableId, value: V) {
		if let Some(entry) = self.handlers.write().await.get_mut(obj) {
			entry.value = value;
		}
	}

	async fn on_left(&self, obj: &QueryableId) {
		// Decide retention while holding the handler write lock so the capture_requests check
		// is serialized against grant_capture, which inserts under the same lock. Otherwise a
		// field exit landing mid-grant could see an empty capture_requests and wrongly drop the
		// entry of a handler that is in the middle of capturing.
		let mut handlers = self.handlers.write().await;
		let Some(handler) = handlers.get(obj).map(|e| e.handler.clone()) else {
			return;
		};
		if self.capture_requests.read().await.contains(&handler) {
			if let Some(entry) = handlers.get_mut(obj) {
				entry.left_query = true;
			}
		} else {
			handlers.remove(obj);
		}
	}
}

#[derive(Debug, Handler)]
pub struct BeamQueryCache(pub QueryCache<RayMarchResult>);
impl BeamQueryHandlerHandler for BeamQueryCache {
	async fn intersected(
		&self,
		_ctx: Context,
		obj: QueryableId,
		field: FieldRefProxy,
		_spatial: SpatialRefProxy,
		interfaces: Vec<QueriedInterface>,
		march_result: RayMarchResult,
	) {
		self.0
			.on_entered(obj, field, interfaces, march_result)
			.await;
	}
	async fn interfaces_changed(
		&self,
		_ctx: Context,
		_obj: QueryableId,
		_interfaces: Vec<QueriedInterface>,
	) {
	}
	async fn moved(&self, _ctx: Context, obj: QueryableId, march_result: RayMarchResult) {
		self.0.on_value_changed(&obj, march_result).await;
	}
	async fn left(&self, _ctx: Context, obj: QueryableId) {
		self.0.on_left(&obj).await;
	}
}

#[derive(Debug, Handler)]
pub struct PointsQueryCache(pub QueryCache<FieldSample>);
impl PointsQueryHandlerHandler for PointsQueryCache {
	async fn entered(
		&self,
		_ctx: Context,
		obj: QueryableId,
		field: FieldRefProxy,
		_spatial: SpatialRefProxy,
		interfaces: Vec<QueriedInterface>,
		sample: FieldSample,
	) {
		self.0.on_entered(obj, field, interfaces, sample).await;
	}
	async fn interfaces_changed(
		&self,
		_ctx: Context,
		_obj: QueryableId,
		_interfaces: Vec<QueriedInterface>,
	) {
	}
	async fn moved(&self, _ctx: Context, obj: QueryableId, sample: FieldSample) {
		self.0.on_value_changed(&obj, sample).await;
	}
	async fn left(&self, _ctx: Context, obj: QueryableId) {
		self.0.on_left(&obj).await;
	}
}

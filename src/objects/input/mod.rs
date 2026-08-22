#![allow(clippy::mutable_key_type)]

// FIX ORDER: 5
// pub mod mouse_pointer;
pub mod oxr_controller;
// FIX ORDER: 5
// pub mod oxr_hand;

use crate::nodes::{
	ProxyExt as _,
	fields::{Field, FieldRef},
	spatial::SpatialRef,
};
use gluon::{Handler, IntoHandler, Liveness, Node, NodeError, RefExt};
use stardust_xr_protocol::{
	field::{FieldRef as FieldRefProxy, FieldSample, RayMarchResult},
	query::{QueriedInterface, QueryableId},
	spatial::SpatialRef as SpatialRefProxy,
	spatial_query::{
		BeamQueryHandler, BeamQueryHandlerHandler, PointsQueryHandler, PointsQueryHandlerHandler,
	},
	suis::{
		DatamapData, InputHandler, InputMethod, InputMethodCapture, InputMethodCaptureHandler,
		InputMethodHandler, SemanticData, SpatialData,
	},
	types::Timestamp,
};
use std::{
	collections::{HashMap, HashSet},
	fmt,
	ops::{Deref, DerefMut},
	sync::{Arc, Mutex, RwLock as StdRwLock},
};
use tokio::sync::{RwLock, mpsc};
use tracing::{debug_span, instrument};

pub struct InputMethodNode<H: InputMethodHandler> {
	pub node: Node<H>,
	pub proxy: InputMethod,
}
impl<H: InputMethodHandler> InputMethodNode<H> {
	pub fn new(handler: impl IntoHandler<H>) -> Result<Self, NodeError> {
		let (node, proxy) = InputMethod::new_node(handler)?;
		Ok(Self {
			node,
			proxy: proxy.into_proxy(),
		})
	}
}
impl<H: InputMethodHandler> DerefMut for InputMethodNode<H> {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.node
	}
}
impl<H: InputMethodHandler> Deref for InputMethodNode<H> {
	type Target = Node<H>;

	fn deref(&self) -> &Self::Target {
		&self.node
	}
}

// ── CachedObject ─────────────────────────────────────────────────────────────

pub struct CachedObject<V: Send + Sync + 'static> {
	pub handler: InputHandler,
	/// None while the get_spatial RPC is in-flight; filtered from dispatch until populated.
	pub spatial: Option<Arc<SpatialRef>>,
	pub field: Arc<FieldRef>,
	pub value: V,
	/// True when the handler has left the spatial query but is retained because it holds a capture.
	pub left_query: bool,
}

impl<V: Send + Sync + 'static> fmt::Debug for CachedObject<V> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("CachedObject").finish_non_exhaustive()
	}
}

// ── QueryCache ───────────────────────────────────────────────────────────────

pub struct QueryCache<V: Send + Sync + 'static> {
	pub objects: Arc<RwLock<HashMap<QueryableId, CachedObject<V>>>>,
	capture_requests: Arc<StdRwLock<HashSet<InputHandler>>>,
}

impl<V: Send + Sync + 'static> fmt::Debug for QueryCache<V> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("QueryCache").finish_non_exhaustive()
	}
}

impl<V: Send + Sync + 'static> QueryCache<V> {
	pub fn new() -> (
		Self,
		Arc<RwLock<HashMap<QueryableId, CachedObject<V>>>>,
		Arc<StdRwLock<HashSet<InputHandler>>>,
	) {
		let objects = Arc::new(RwLock::new(HashMap::new()));
		let capture_requests = Arc::new(StdRwLock::new(HashSet::new()));
		(
			Self {
				objects: objects.clone(),
				capture_requests: capture_requests.clone(),
			},
			objects,
			capture_requests,
		)
	}

	pub async fn on_entered(
		&self,
		obj: QueryableId,
		field: FieldRefProxy,
		_spatial: SpatialRefProxy,
		interfaces: Vec<QueriedInterface>,
		value: V,
	) {
		let Some(interface) = interfaces.first() else {
			return;
		};
		if interface.interface_id != InputHandler::QUERY_INTERFACE {
			return;
		}
		let Some(field) = field.owned() else { return };
		let handler = InputHandler::from_ref(interface.interface.clone());

		{
			let mut objects = self.objects.write().await;
			if let Some(entry) = objects.get_mut(&obj) {
				// Re-entered the query — the beam flaps across a field boundary, so on_entered
				// fires again for an object we already track. Keep the existing spatial: nulling
				// it would momentarily drop a captured/retained handler out of dispatch and emit
				// a spurious input_left. Just refresh field/value and clear the left flag. No need
				// to re-run get_spatial; the handler's reference spatial is stable.
				entry.field = field;
				entry.value = value;
				entry.left_query = false;
				return;
			}

			// First time seeing this object: insert a sentinel (spatial: None) before the async
			// get_spatial RPC so on_left can find and remove this entry even if it fires while
			// get_spatial is in-flight.
			objects.insert(
				obj.clone(),
				CachedObject {
					handler: handler.clone(),
					spatial: None,
					field,
					value,
					left_query: false,
				},
			);
		}

		let Ok(ref_space) = handler.get_spatial().await else {
			self.objects.write().await.remove(&obj);
			return;
		};
		let Some(spatial) = ref_space.owned() else {
			self.objects.write().await.remove(&obj);
			return;
		};

		// Populate spatial only if the sentinel we inserted is still there (on_left may have
		// removed it while we were awaiting).
		if let Some(entry) = self.objects.write().await.get_mut(&obj)
			&& entry.spatial.is_none()
		{
			entry.spatial = Some(spatial);
		}
	}

	pub async fn on_value_changed(&self, obj: &QueryableId, new_value: V) {
		if let Some(entry) = self.objects.write().await.get_mut(obj) {
			entry.value = new_value;
		}
	}

	pub async fn on_left(&self, obj: &QueryableId) {
		// Decide retention while holding the cache write lock so the capture_requests check is
		// serialized against grant_capture (which inserts under the same lock). Otherwise a
		// field-exit landing mid-grant could observe an empty capture_requests and wrongly drop
		// the entry of a handler that is in the process of capturing.
		let mut objects = self.objects.write().await;
		let Some(handler) = objects.get(obj).map(|e| e.handler.clone()) else {
			return;
		};
		let is_captured = self.capture_requests.read().unwrap().contains(&handler);
		if is_captured {
			// Keep the entry so the captured handler keeps receiving input; just flag it.
			if let Some(entry) = objects.get_mut(obj) {
				entry.left_query = true;
			}
		} else {
			objects.remove(obj);
		}
	}
}

// ── BeamQueryCache ────────────────────────────────────────────────────────────

#[derive(Debug, Handler)]
pub struct BeamQueryCache(pub QueryCache<RayMarchResult>);

impl BeamQueryHandlerHandler for BeamQueryCache {
	async fn intersected(
		&self,
		_ctx: gluon::Context,
		obj: QueryableId,
		field: FieldRefProxy,
		spatial: SpatialRefProxy,
		interfaces: Vec<QueriedInterface>,
		march_result: RayMarchResult,
	) {
		self.0
			.on_entered(obj, field, spatial, interfaces, march_result)
			.await;
	}

	async fn interfaces_changed(
		&self,
		_ctx: gluon::Context,
		_obj: QueryableId,
		_interfaces: Vec<QueriedInterface>,
	) {
	}

	async fn moved(&self, _ctx: gluon::Context, obj: QueryableId, march_result: RayMarchResult) {
		self.0.on_value_changed(&obj, march_result).await;
	}

	async fn left(&self, _ctx: gluon::Context, obj: QueryableId) {
		self.0.on_left(&obj).await;
	}
}

// ── PointsQueryCache ──────────────────────────────────────────────────────────

#[derive(Debug, Handler)]
pub struct PointsQueryCache(pub QueryCache<FieldSample>);

impl PointsQueryHandlerHandler for PointsQueryCache {
	async fn entered(
		&self,
		_ctx: gluon::Context,
		obj: QueryableId,
		field: FieldRefProxy,
		spatial: SpatialRefProxy,
		interfaces: Vec<QueriedInterface>,
		sample: FieldSample,
	) {
		self.0
			.on_entered(obj, field, spatial, interfaces, sample)
			.await;
	}

	async fn interfaces_changed(
		&self,
		_ctx: gluon::Context,
		_obj: QueryableId,
		_interfaces: Vec<QueriedInterface>,
	) {
	}

	async fn moved(&self, _ctx: gluon::Context, obj: QueryableId, sample: FieldSample) {
		self.0.on_value_changed(&obj, sample).await;
	}

	async fn left(&self, _ctx: gluon::Context, obj: QueryableId) {
		self.0.on_left(&obj).await;
	}
}

// ── ActiveTracker ─────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ActiveTracker {
	active: HashSet<InputHandler>,
}

impl ActiveTracker {
	pub fn update(
		&mut self,
		new: HashSet<InputHandler>,
	) -> (HashSet<InputHandler>, HashSet<InputHandler>) {
		let added = new.difference(&self.active).cloned().collect();
		let removed = self.active.difference(&new).cloned().collect();
		self.active = new;
		(added, removed)
	}
}

// ── InputSource trait ─────────────────────────────────────────────────────────

pub trait InputSource {
	type QueryValue: Send + Sync + 'static;

	fn order_handlers_and_captures(
		&self,
		objects: &HashMap<QueryableId, CachedObject<Self::QueryValue>>,
		capture_requests: &HashSet<InputHandler>,
	) -> (Vec<InputHandler>, Option<InputHandler>);

	fn spatial_data(&self, handler_spatial: &SpatialRef, handler_field: &Field) -> SpatialData;

	fn datamap(&self) -> HashMap<String, DatamapData>;
}

// ── CaptureGuard ─────────────────────────────────────────────────────────────

#[derive(Debug, Handler)]
pub struct CaptureGuard {
	handler: InputHandler,
	release_tx: mpsc::UnboundedSender<InputHandler>,
}
impl InputMethodCaptureHandler for CaptureGuard {}
impl Drop for CaptureGuard {
	fn drop(&mut self) {
		let _ = self.release_tx.send(self.handler.clone());
	}
}

// ── InputSender ───────────────────────────────────────────────────────────────

pub struct InputSender<V: Send + Sync + 'static> {
	pub cache: Arc<RwLock<HashMap<QueryableId, CachedObject<V>>>>,
	pub capture_requests: Arc<StdRwLock<HashSet<InputHandler>>>,
	pub active_capture: RwLock<Option<InputHandler>>,
	release_tx: mpsc::UnboundedSender<InputHandler>,
	release_rx: Mutex<mpsc::UnboundedReceiver<InputHandler>>,
	tracker: Mutex<ActiveTracker>,
}

impl<V: Send + Sync + 'static> fmt::Debug for InputSender<V> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("InputSender").finish_non_exhaustive()
	}
}

impl<V: Send + Sync + 'static> InputSender<V> {
	pub fn new(
		cache: Arc<RwLock<HashMap<QueryableId, CachedObject<V>>>>,
		capture_requests: Arc<StdRwLock<HashSet<InputHandler>>>,
	) -> Self {
		let (release_tx, release_rx) = mpsc::unbounded_channel();
		Self {
			cache,
			capture_requests,
			active_capture: RwLock::new(None),
			release_tx,
			release_rx: Mutex::new(release_rx),
			tracker: Mutex::new(ActiveTracker::default()),
		}
	}

	pub async fn grant_capture(&self, handler: InputHandler) -> Option<InputMethodCapture> {
		// Hold the cache write lock across the membership check and the capture_requests insert
		// so on_left (which checks capture_requests under the same lock) can't slip in between
		// and drop this handler's entry on a concurrent field-exit. See on_left for the other
		// half of this serialization.
		{
			let cache = self.cache.write().await;
			if !cache.values().any(|e| e.handler == handler) {
				return None;
			}
			self.capture_requests
				.write()
				.unwrap()
				.insert(handler.clone());
		}
		let capture = InputMethodCapture::new_service(CaptureGuard {
			handler,
			release_tx: self.release_tx.clone(),
		})
		// TODO: get rid of this unwrap
		.unwrap()
		.into_proxy();
		Some(capture)
	}

	/// Force-release the active capture, as if the capturing client dropped its
	/// guard. Takes effect on the next `send` (which drains the release channel).
	pub fn stop_active_capture(&self) {
		if let Some(handler) = self.active_capture.blocking_read().clone() {
			let _ = self.release_tx.send(handler);
		}
	}

	#[instrument(name = "send input sender", level = "debug", skip_all)]
	pub fn send(
		&self,
		source: &impl InputSource<QueryValue = V>,
		method: InputMethod,
		ts: Timestamp,
	) {
		// Drain released captures (CaptureGuard dropped by client).
		debug_span!("drain released captures").in_scope(|| {
			let mut rx = self.release_rx.lock().unwrap();
			while let Ok(released) = rx.try_recv() {
				self.capture_requests.write().unwrap().remove(&released);
				self.cache
					.blocking_write()
					.retain(|_, e| !(e.left_query && e.handler == released));
				let mut cap = self.active_capture.blocking_write();
				if cap.as_ref() == Some(&released) {
					cap.take();
				}
			}
		});

		// Sweep handlers whose client died without on_left being called (e.g. silent
		// binder drop, missed notification). Runs every frame so the cleanup is
		// eventually consistent regardless of whether on_left fires.
		debug_span!("drain dead input handler clients").in_scope(|| {
			let has_dead = self
				.cache
				.blocking_read()
				.values()
				.any(|e| !e.handler.alive());
			if has_dead {
				let mut cap = self.capture_requests.write().unwrap();
				self.cache.blocking_write().retain(|_, e| {
					let alive = e.handler.alive();
					if !alive {
						cap.remove(&e.handler);
					}
					alive
				});
			}
		});

		// Snapshot capture_requests immediately so the std lock is never held
		// across cache reads/writes (which could block tokio worker threads).
		let capture_requests: HashSet<InputHandler> = self.capture_requests.read().unwrap().clone();

		let objects = self.cache.blocking_read();

		let (handler_order, capture) = debug_span!("order handlers and captures")
			.in_scope(|| source.order_handlers_and_captures(&objects, &capture_requests));

		let dispatch: Vec<(InputHandler, SpatialData, SemanticData)> =
			debug_span!("prepare input for dispatch").in_scope(|| {
				handler_order
					.iter()
					.enumerate()
					.filter_map(|(i, handler)| {
						let entry = objects.values().find(|e| &e.handler == handler)?;
						let spatial_data =
							source.spatial_data(entry.spatial.as_deref()?, &entry.field.data);
						let datamap = source.datamap();
						let semantic_data = SemanticData {
							datamap,
							order: i as u32,
							captured: capture.as_ref().is_some_and(|c| c == handler),
						};
						Some((handler.clone(), spatial_data, semantic_data))
					})
					.collect()
			});

		let new_set: HashSet<InputHandler> = handler_order.into_iter().collect();
		let (added, removed) = debug_span!("track added/removed")
			.in_scope(|| self.tracker.lock().unwrap().update(new_set));

		drop(objects);

		let _guard = debug_span!("send update events to objects", count = dispatch.len()).entered();
		for (handler, spatial_data, semantic_data) in dispatch {
			if added.contains(&handler) {
				handler.input_gained(method.clone(), ts, spatial_data, semantic_data);
			} else {
				handler.input_updated(method.clone(), ts, spatial_data, semantic_data);
			}
		}
		for handler in removed {
			handler.input_left(method.clone(), ts);
		}
	}
}

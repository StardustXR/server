#![allow(clippy::mutable_key_type)]

pub mod mouse_pointer;
// pub mod oxr_controller;
pub mod oxr_hand;

use crate::{
	PION,
	nodes::{
		ProxyExt as _,
		fields::{Field, FieldRef},
		spatial::SpatialRef,
	},
};
use gluon::{Handler, ObjectRef, ToObjectOrRef as _};
use stardust_xr_protocol::{
	field::FieldRef as FieldRefProxy,
	query::{QueriedInterface, QueryableObjectRef},
	spatial::SpatialRef as SpatialRefProxy,
	spatial_query::{
		BeamQueryHandler, BeamQueryHandlerHandler, PointsQueryHandler, PointsQueryHandlerHandler,
	},
	suis::{
		DatamapData, InputHandler, InputMethod, InputMethodCapture, InputMethodCaptureHandler,
		SemanticData, SpatialData,
	},
	types::Timestamp,
};
use std::{
	collections::{HashMap, HashSet},
	fmt,
	sync::{Arc, Mutex, RwLock as StdRwLock},
};
use tokio::sync::{RwLock, mpsc};
use tracing::{debug_span, instrument};

// ── Value types ──────────────────────────────────────────────────────────────

pub struct BeamValue {
	pub deepest_point_distance: f32,
	pub distance: f32,
}

// ── CachedObject ─────────────────────────────────────────────────────────────

pub struct CachedObject<V: Send + Sync + 'static> {
	pub handler: InputHandler,
	/// None while the get_spatial RPC is in-flight; filtered from dispatch until populated.
	pub spatial: Option<ObjectRef<SpatialRef>>,
	pub field: ObjectRef<FieldRef>,
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
	pub objects: Arc<RwLock<HashMap<QueryableObjectRef, CachedObject<V>>>>,
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
		Arc<RwLock<HashMap<QueryableObjectRef, CachedObject<V>>>>,
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
		obj: QueryableObjectRef,
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
		let handler = InputHandler::from_object_or_ref(interface.interface.clone());

		// Insert a sentinel (spatial: None) before the async RPC so that on_left can find
		// and remove this entry even if it fires while get_spatial is in-flight.
		self.objects.write().await.insert(
			obj.clone(),
			CachedObject {
				handler: handler.clone(),
				spatial: None,
				field,
				value,
				left_query: false,
			},
		);

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
		if let Some(entry) = self.objects.write().await.get_mut(&obj) {
			if entry.spatial.is_none() {
				entry.spatial = Some(spatial);
			}
		}
	}

	pub async fn on_value_changed(&self, obj: &QueryableObjectRef, new_value: V) {
		if let Some(entry) = self.objects.write().await.get_mut(obj) {
			entry.value = new_value;
		}
	}

	pub async fn on_left(&self, obj: &QueryableObjectRef) {
		// Check whether this handler holds an active capture before dropping its entry.
		// Read the handler out while holding a read lock, then check capture_requests
		// (a std lock, so no await needed), then promote to write to either mark as
		// left_query or actually remove.
		let handler = {
			let objects = self.objects.read().await;
			objects.get(obj).map(|e| e.handler.clone())
		};

		let is_captured = handler
			.as_ref()
			.is_some_and(|h| self.capture_requests.read().unwrap().contains(h));

		let mut objects = self.objects.write().await;
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
pub struct BeamQueryCache(pub QueryCache<BeamValue>);

impl BeamQueryHandlerHandler for BeamQueryCache {
	async fn intersected(
		&self,
		_ctx: gluon::Context,
		obj: QueryableObjectRef,
		field: FieldRefProxy,
		spatial: SpatialRefProxy,
		interfaces: Vec<QueriedInterface>,
		deepest_point_distance: f32,
		distance: f32,
	) {
		self.0
			.on_entered(
				obj,
				field,
				spatial,
				interfaces,
				BeamValue {
					deepest_point_distance,
					distance,
				},
			)
			.await;
	}

	async fn interfaces_changed(
		&self,
		_ctx: gluon::Context,
		_obj: QueryableObjectRef,
		_interfaces: Vec<QueriedInterface>,
	) {
	}

	async fn moved(
		&self,
		_ctx: gluon::Context,
		obj: QueryableObjectRef,
		deepest_point_distance: f32,
		distance: f32,
	) {
		self.0
			.on_value_changed(
				&obj,
				BeamValue {
					deepest_point_distance,
					distance,
				},
			)
			.await;
	}

	async fn left(&self, _ctx: gluon::Context, obj: QueryableObjectRef) {
		self.0.on_left(&obj).await;
	}
}

// ── PointsQueryCache ──────────────────────────────────────────────────────────

#[derive(Debug, Handler)]
pub struct PointsQueryCache(pub QueryCache<f32>);

impl PointsQueryHandlerHandler for PointsQueryCache {
	async fn entered(
		&self,
		_ctx: gluon::Context,
		obj: QueryableObjectRef,
		field: FieldRefProxy,
		spatial: SpatialRefProxy,
		interfaces: Vec<QueriedInterface>,
		distance: f32,
	) {
		self.0
			.on_entered(obj, field, spatial, interfaces, distance)
			.await;
	}

	async fn interfaces_changed(
		&self,
		_ctx: gluon::Context,
		_obj: QueryableObjectRef,
		_interfaces: Vec<QueriedInterface>,
	) {
	}

	async fn moved(&self, _ctx: gluon::Context, obj: QueryableObjectRef, distance: f32) {
		self.0.on_value_changed(&obj, distance).await;
	}

	async fn left(&self, _ctx: gluon::Context, obj: QueryableObjectRef) {
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
		objects: &HashMap<QueryableObjectRef, CachedObject<Self::QueryValue>>,
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
	pub cache: Arc<RwLock<HashMap<QueryableObjectRef, CachedObject<V>>>>,
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
		cache: Arc<RwLock<HashMap<QueryableObjectRef, CachedObject<V>>>>,
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
		if !self
			.cache
			.read()
			.await
			.values()
			.any(|e| e.handler == handler)
		{
			return None;
		}
		self.capture_requests
			.write()
			.unwrap()
			.insert(handler.clone());
		let guard = PION
			.register_object(CaptureGuard {
				handler,
				release_tx: self.release_tx.clone(),
			})
			.to_service();
		let capture = InputMethodCapture::from_handler(&guard);
		Some(capture)
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
				.any(|e| !e.handler.to_binder_object_or_ref().alive());
			if has_dead {
				let mut cap = self.capture_requests.write().unwrap();
				self.cache.blocking_write().retain(|_, e| {
					let alive = e.handler.to_binder_object_or_ref().alive();
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
						let spatial_data = source.spatial_data(entry.spatial.as_ref().map(|s| &**s)?, &entry.field.data);
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

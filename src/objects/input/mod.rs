pub mod mouse_pointer;
// pub mod oxr_controller;
pub mod oxr_hand;

use crate::nodes::{
	ProxyExt as _,
	fields::{Field, FieldRef},
	spatial::SpatialRef,
};
use gluon::{Handler, ObjectRef};
use stardust_xr_protocol::{
	field::FieldRef as FieldRefProxy,
	query::{QueriedInterface, QueryableObjectRef},
	spatial::SpatialRef as SpatialRefProxy,
	spatial_query::{
		BeamQueryHandler, BeamQueryHandlerHandler, PointsQueryHandler, PointsQueryHandlerHandler,
	},
	suis::{DatamapData, InputHandler, InputMethod, SemanticData, SpatialData},
	types::Timestamp,
};
use std::{
	collections::{HashMap, HashSet},
	fmt,
	sync::{Arc, Mutex, RwLock as StdRwLock},
};
use tokio::sync::RwLock;

// ── Value types ──────────────────────────────────────────────────────────────

pub struct BeamValue {
	pub deepest_point_distance: f32,
	pub distance: f32,
}

// ── CachedObject ─────────────────────────────────────────────────────────────

pub struct CachedObject<V: Send + Sync + 'static> {
	pub handler: InputHandler,
	pub spatial: ObjectRef<SpatialRef>,
	pub field: ObjectRef<FieldRef>,
	pub suggested_bindings: HashMap<String, Vec<String>>,
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
		spatial: SpatialRefProxy,
		interfaces: Vec<QueriedInterface>,
		value: V,
	) {
		let Some(interface) = interfaces.first() else {
			return;
		};
		if interface.interface_id != InputHandler::QUERY_INTERFACE {
			return;
		}
		let Some(spatial) = spatial.owned() else {
			return;
		};
		let Some(field) = field.owned() else { return };
		let handler = InputHandler::from_object_or_ref(interface.interface.clone());

		let suggested_bindings = handler.suggested_bindings().await.unwrap_or_default();

		self.objects.write().await.insert(
			obj,
			CachedObject {
				handler,
				spatial,
				field,
				suggested_bindings,
				value,
				left_query: false,
			},
		);
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

	fn datamap(
		&self,
		suggested_bindings: &HashMap<String, Vec<String>>,
	) -> HashMap<String, DatamapData>;
}

// ── InputSender ───────────────────────────────────────────────────────────────

pub struct InputSender<V: Send + Sync + 'static> {
	pub cache: Arc<RwLock<HashMap<QueryableObjectRef, CachedObject<V>>>>,
	pub capture_requests: Arc<StdRwLock<HashSet<InputHandler>>>,
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
		Self {
			cache,
			capture_requests,
			tracker: Mutex::new(ActiveTracker::default()),
		}
	}

	pub async fn request_capture(&self, handler: InputHandler) {
		if self
			.cache
			.read()
			.await
			.values()
			.any(|e| e.handler == handler)
		{
			self.capture_requests.write().unwrap().insert(handler);
		}
	}

	pub async fn release_capture(&self, handler: &InputHandler) {
		self.capture_requests.write().unwrap().remove(handler);
		// Remove left_query entries for this handler; they were retained only for the capture.
		self.cache
			.write()
			.await
			.retain(|_, e| !(e.left_query && &e.handler == handler));
	}

	pub fn send(
		&self,
		source: &impl InputSource<QueryValue = V>,
		method: InputMethod,
		ts: Timestamp,
	) {
		// Snapshot capture_requests immediately so the std lock is never held
		// across cache reads/writes (which could block tokio worker threads).
		let capture_requests: HashSet<InputHandler> =
			self.capture_requests.read().unwrap().clone();

		// Clean up any left_query entries that lost their capture between on_left and
		// release_capture running (race condition). This keeps the objects map consistent.
		{
			let has_dangling = self
				.cache
				.blocking_read()
				.values()
				.any(|e| e.left_query && !capture_requests.contains(&e.handler));
			if has_dangling {
				self.cache
					.blocking_write()
					.retain(|_, e| !e.left_query || capture_requests.contains(&e.handler));
			}
		}

		let objects = self.cache.blocking_read();

		let (handler_order, capture) =
			source.order_handlers_and_captures(&objects, &capture_requests);

		let dispatch: Vec<(InputHandler, SpatialData, SemanticData)> = handler_order
			.iter()
			.enumerate()
			.filter_map(|(i, handler)| {
				let entry = objects.values().find(|e| &e.handler == handler)?;
				let spatial_data = source.spatial_data(&entry.spatial, &entry.field.data);
				let datamap = source.datamap(&entry.suggested_bindings);
				let semantic_data = SemanticData {
					datamap,
					order: i as u32,
					captured: capture.as_ref().is_some_and(|c| c == handler),
				};
				Some((handler.clone(), spatial_data, semantic_data))
			})
			.collect();

		let new_set: HashSet<InputHandler> = handler_order.into_iter().collect();
		let (added, removed) = self.tracker.lock().unwrap().update(new_set);

		drop(objects);

		tokio::spawn(async move {
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
		});
	}
}

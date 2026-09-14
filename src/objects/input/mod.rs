#![allow(clippy::mutable_key_type)]

mod localize;
pub use localize::localize;
pub mod mouse_pointer;
pub mod oxr_controller;
pub mod oxr_hand;
mod query_cache;
pub use query_cache::*;

use crate::{
	nodes::{ProxyExt as _, spatial::Spatial},
	query::spatial_query::SpatialQueryInterface,
};
use gluon_ipc::{Context, Handler, Liveness, Node, NodeError, RefExt};
use stardust_xr_protocol::{
	field::{FieldSample, RayMarchResult},
	query::InterfaceDependency,
	query::QueryableId,
	spatial::SpatialRef as SpatialRefProxy,
	spatial_query::{
		BeamQuery, BeamQueryHandle, BeamQueryHandler, Point, PointsQuery, PointsQueryHandle,
		PointsQueryHandler, SpatialQueryInterface as SpatialQueryInterfaceProxy,
	},
	suis::{
		DatamapData, InputDataType, InputHandler, InputMethod as InputMethodProxy,
		InputMethodCapture, InputMethodCaptureHandler, InputMethodHandler, SemanticData,
		SpatialData,
	},
	types::{Timestamp, Vec2F, Vec3F},
};
use std::{
	collections::{HashMap, HashSet},
	fmt::{Debug, Formatter},
	future::Future,
	ops::Deref,
	sync::{Arc, Mutex, OnceLock},
};
use tokio::sync::{RwLock, mpsc, watch};
use tracing::error;

fn handler_dependency() -> InterfaceDependency {
	InterfaceDependency {
		id: InputHandler::QUERY_INTERFACE.to_string(),
		optional: false,
	}
}

/// The device behind an [`InputMethod`].
///
/// Describe the device in its own space and [`localize`] puts it in each handler's space,
/// filling in every distance on the way.
pub trait InputMethodHelper: Send + Sync + 'static {
	/// what the query reports about each handler: [`RayMarchResult`] for a beam,
	/// [`FieldSample`] for points
	type QueryValue: Send + Sync + 'static;

	/// which handlers get input this cycle, closest first, and which capture requester wins
	///
	/// runs while the cache is locked for reading, so the query can't add or drop handlers
	/// until it finishes. [`order_by_distance`] is what all the built-in devices use
	fn order_handlers_and_captures(
		&self,
		handlers: &HashMap<QueryableId, CachedHandler<Self::QueryValue>>,
		capture_requests: &HashSet<InputHandler>,
		active_capture: Option<&InputHandler>,
	) -> impl Future<Output = (Vec<InputHandler>, Option<InputHandler>)> + Send + Sync;

	/// this method's shape in its own space, None to send nothing this cycle
	///
	/// `time` is when the input is for, which devices that can locate themselves in the past
	/// (an OpenXR space, say) should honor, the rest can ignore
	fn input_data(
		&self,
		time: Timestamp,
	) -> impl Future<Output = Option<InputDataType>> + Send + Sync;

	fn datamap(&self) -> impl Future<Output = HashMap<String, DatamapData>> + Send + Sync;

	/// a capture was just handed out, with the requesting client's context
	fn capture_granted(
		&self,
		_ctx: &Context,
		_handler: &InputHandler,
	) -> impl Future<Output = ()> + Send + Sync {
		async {}
	}
}

#[derive(Default)]
struct ActiveTracker {
	active: HashSet<InputHandler>,
}
impl ActiveTracker {
	fn update(
		&mut self,
		new: HashSet<InputHandler>,
	) -> (HashSet<InputHandler>, HashSet<InputHandler>) {
		let added = new.difference(&self.active).cloned().collect();
		let removed = self.active.difference(&new).cloned().collect();
		self.active = new;
		(added, removed)
	}
}

#[derive(Debug, Handler)]
struct CaptureGuard {
	handler: InputHandler,
	release_tx: mpsc::UnboundedSender<InputHandler>,
}
impl InputMethodCaptureHandler for CaptureGuard {}
impl Drop for CaptureGuard {
	fn drop(&mut self) {
		let _ = self.release_tx.send(self.handler.clone());
	}
}

#[derive(Handler)]
pub struct InputMethod<S: InputMethodHelper> {
	helper: S,
	cache: QueryCache<S::QueryValue>,
	spatial: Arc<Spatial>,
	active_capture: RwLock<Option<InputHandler>>,
	release_tx: mpsc::UnboundedSender<InputHandler>,
	release_rx: Mutex<mpsc::UnboundedReceiver<InputHandler>>,
	tracker: Mutex<ActiveTracker>,
	// bare proxy, not a LocalRef: a LocalRef here would be an Arc cycle back into this handler
	proxy: OnceLock<InputMethodProxy>,
	// the query lives exactly as long as its handler node, so holding it here is what makes
	// dropping the method drop the query
	query: OnceLock<Box<dyn Send + Sync>>,
}
impl<S: InputMethodHelper> Debug for InputMethod<S> {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("InputMethod").finish_non_exhaustive()
	}
}
impl<S: InputMethodHelper> Deref for InputMethod<S> {
	type Target = S;

	fn deref(&self) -> &S {
		&self.helper
	}
}

impl<S: InputMethodHelper> InputMethod<S> {
	fn finish(
		helper: S,
		cache: QueryCache<S::QueryValue>,
		spatial: Arc<Spatial>,
		query: impl Send + Sync + 'static,
	) -> Result<(Node<Self>, InputMethodProxy), NodeError> {
		let (release_tx, release_rx) = mpsc::unbounded_channel();
		let (node, method) = InputMethodProxy::new_node(Self {
			helper,
			cache,
			spatial,
			active_capture: RwLock::new(None),
			release_tx,
			release_rx: Mutex::new(release_rx),
			tracker: Mutex::default(),
			proxy: OnceLock::new(),
			query: OnceLock::new(),
		})?;
		let proxy = method.into_proxy();
		let _ = node.proxy.set(proxy.clone());
		let _ = node.query.set(Box::new(query));
		Ok((node, proxy))
	}

	pub fn helper(&self) -> &S {
		&self.helper
	}

	pub fn cache(&self) -> &QueryCache<S::QueryValue> {
		&self.cache
	}

	/// the space this method lives in: what the query is relative to, and what
	/// [`InputMethodHelper::input_data`] is relative to
	pub fn spatial(&self) -> &Arc<Spatial> {
		&self.spatial
	}

	/// the handler currently winning the capture, if any
	pub async fn active_capture(&self) -> Option<InputHandler> {
		self.active_capture.read().await.clone()
	}

	pub fn active_capture_blocking(&self) -> Option<InputHandler> {
		self.active_capture.blocking_read().clone()
	}

	/// force-release the active capture, as if the capturing client dropped its guard
	///
	/// takes effect on the next [`Self::send`], which is what drains the release channel
	pub fn stop_active_capture(&self) {
		if let Some(handler) = self.active_capture_blocking() {
			let _ = self.release_tx.send(handler);
		}
	}

	async fn grant_capture(&self, handler: InputHandler) -> Option<InputMethodCapture> {
		// Hold the handler write lock across the membership check and the capture_requests
		// insert so on_left, which checks capture_requests under the same lock, can't slip in
		// and drop this handler's entry on a concurrent field exit.
		{
			let handlers = self.cache.handlers_mut().await;
			if !handlers.values().any(|e| e.handler == handler) {
				return None;
			}
			self.cache
				.capture_requests_mut()
				.await
				.insert(handler.clone());
		}
		let capture = InputMethodCapture::new_service(CaptureGuard {
			handler,
			release_tx: self.release_tx.clone(),
		})
		.ok()?;
		Some(capture.into_proxy())
	}

	async fn drain_released_captures(&self) {
		let released: Vec<InputHandler> = {
			let mut rx = self.release_rx.lock().unwrap();
			std::iter::from_fn(|| rx.try_recv().ok()).collect()
		};
		for handler in released {
			self.cache.capture_requests_mut().await.remove(&handler);
			self.cache
				.handlers_mut()
				.await
				.retain(|_, e| !(e.left_query && e.handler == handler));
			let mut capture = self.active_capture.write().await;
			if capture.as_ref() == Some(&handler) {
				capture.take();
			}
		}
	}

	/// sweep handlers whose client died without the query ever saying they left
	async fn drain_dead_handlers(&self) {
		if !self
			.cache
			.handlers()
			.await
			.values()
			.any(|e| !e.handler.alive())
		{
			return;
		}
		let mut capture_requests = self.cache.capture_requests_mut().await;
		self.cache.handlers_mut().await.retain(|_, e| {
			let alive = e.handler.alive();
			if !alive {
				capture_requests.remove(&e.handler);
			}
			alive
		});
	}

	/// one dispatch cycle: work out who gets what and send it
	pub async fn send(&self, time: Timestamp) {
		let Some(method) = self.proxy.get() else {
			return;
		};
		self.drain_released_captures().await;
		self.drain_dead_handlers().await;

		// a handler whose get_spatial is still in flight is skipped rather than kept in
		// order, so its first real send is always an input_gained
		let (capture, targets) = {
			let handlers = self.cache.handlers().await;
			let capture_requests = self.cache.capture_requests().await.clone();
			let active = self.active_capture.read().await.clone();
			let (order, capture) = self
				.helper
				.order_handlers_and_captures(&handlers, &capture_requests, active.as_ref())
				.await;
			let targets: Vec<_> = order
				.into_iter()
				.filter_map(|handler| {
					let entry = handlers.values().find(|e| e.handler == handler)?;
					Some((handler, entry.spatial.clone()?, entry.field.clone()))
				})
				.collect();
			(capture, targets)
		};
		*self.active_capture.write().await = capture.clone();

		let (datamap, input) = tokio::join!(self.helper.datamap(), self.helper.input_data(time));

		// an untracked device dispatches to nobody rather than returning early, so whoever was
		// active still gets their input_left
		let mut dispatch: Vec<(InputHandler, SpatialData, SemanticData)> = Vec::new();
		if let Some(input) = input {
			dispatch = targets
				.into_iter()
				.filter_map(|(handler, spatial, field)| {
					Some((handler, localize(&self.spatial, &spatial, &field, &input)?))
				})
				.enumerate()
				.map(|(order, (handler, spatial))| {
					let semantic = SemanticData {
						datamap: datamap.clone(),
						order: order as u32,
						captured: capture.as_ref() == Some(&handler),
					};
					(handler, spatial, semantic)
				})
				.collect();
		}

		let (added, removed) = self.tracker.lock().unwrap().update(
			dispatch
				.iter()
				.map(|(handler, ..)| handler.clone())
				.collect(),
		);

		for (handler, spatial, semantic) in dispatch {
			if added.contains(&handler) {
				handler.input_gained(method.clone(), time, spatial, semantic);
			} else {
				handler.input_updated(method.clone(), time, spatial, semantic);
			}
		}
		for handler in removed {
			handler.input_left(method.clone(), time);
		}
	}
}

impl<S: InputMethodHelper<QueryValue = RayMarchResult>> InputMethod<S> {
	#[allow(clippy::too_many_arguments)]
	pub fn new_beam(
		helper: S,
		spatial: Arc<Spatial>,
		reference_spatial: SpatialRefProxy,
		origin: Vec3F,
		direction: Vec3F,
		max_length: f32,
		margin: f32,
	) -> Result<(Node<Self>, InputMethodProxy, Arc<OnceLock<BeamQueryHandle>>), NodeError> {
		let cache = QueryCache::default();
		let (query_node, query) = BeamQueryHandler::new_node(BeamQueryCache(cache.clone()))?;
		let handle = Arc::new(OnceLock::new());
		tokio::spawn({
			let handle = handle.clone();
			async move {
				let result = spatial_query_interface()
					.beam_query(BeamQuery {
						handler: query.into_proxy(),
						interfaces: vec![handler_dependency()],
						reference_spatial,
						origin,
						direction,
						max_length,
						margin,
					})
					.await;
				match result {
					Ok(Ok(created)) => {
						let _ = handle.set(created);
					}
					Ok(Err(err)) => error!("failed to create beam query: {err}"),
					Err(err) => error!("failed to create beam query: {err}"),
				}
			}
		});
		let (node, proxy) = Self::finish(helper, cache, spatial, query_node)?;
		Ok((node, proxy, handle))
	}
}

impl<S: InputMethodHelper<QueryValue = FieldSample>> InputMethod<S> {
	pub fn new_points(
		helper: S,
		spatial: Arc<Spatial>,
		reference_spatial: SpatialRefProxy,
		points: Vec<Point>,
	) -> Result<
		(
			Node<Self>,
			InputMethodProxy,
			Arc<OnceLock<PointsQueryHandle>>,
		),
		NodeError,
	> {
		let cache = QueryCache::default();
		let (query_node, query) = PointsQueryHandler::new_node(PointsQueryCache(cache.clone()))?;
		let handle = Arc::new(OnceLock::new());
		tokio::spawn({
			let handle = handle.clone();
			async move {
				let result = spatial_query_interface()
					.points_query(PointsQuery {
						handler: query.into_proxy(),
						interfaces: vec![handler_dependency()],
						reference_spatial,
						points,
					})
					.await;
				match result {
					Ok(Ok(created)) => {
						let _ = handle.set(created);
					}
					Ok(Err(err)) => error!("failed to create points query: {err}"),
					Err(err) => error!("failed to create points query: {err}"),
				}
			}
		});
		let (node, proxy) = Self::finish(helper, cache, spatial, query_node)?;
		Ok((node, proxy, handle))
	}
}

impl<S: InputMethodHelper> InputMethodHandler for InputMethod<S> {
	async fn request_capture(
		&self,
		ctx: Context,
		handler: InputHandler,
	) -> Option<InputMethodCapture> {
		let capture = self.grant_capture(handler.clone()).await?;
		self.helper.capture_granted(&ctx, &handler).await;
		Some(capture)
	}

	async fn get_spatial_data(
		&self,
		_ctx: Context,
		handler: InputHandler,
		time: Timestamp,
	) -> Option<SpatialData> {
		if self
			.active_capture
			.read()
			.await
			.as_ref()
			.is_some_and(|c| c != &handler)
		{
			return None;
		}
		let cached = self
			.cache
			.handlers()
			.await
			.values()
			.find(|e| e.handler == handler)
			.and_then(|e| Some((e.spatial.clone()?, e.field.clone())));
		// a handler outside the query can still ask, it just costs two round trips
		let (spatial, field) = match cached {
			Some(pair) => pair,
			None => (
				handler.get_spatial().await.ok()?.owned()?,
				handler.get_field().await.ok()?.owned()?.data.clone(),
			),
		};
		let input = self.helper.input_data(time).await?;
		localize(&self.spatial, &spatial, &field, &input)
	}
}

/// A node the spatial query interface is reachable through for the lifetime of the query
/// it creates, since [`RefExt::new_service`] hands the node's lifetime to its refs.
fn spatial_query_interface() -> SpatialQueryInterfaceProxy {
	SpatialQueryInterfaceProxy::new_service(SpatialQueryInterface::new(&Arc::default()))
		.expect("failed to create the spatial query interface node")
		.into_proxy()
}

/// Drives an [`InputMethod`] from whatever loop the device's data arrives on.
///
/// Frames are coalesced, so a send that outlasts its frame drops the timestamps stacked up
/// behind it rather than queueing sends that are already stale.
pub struct FrameDriver(watch::Sender<Timestamp>);
impl FrameDriver {
	pub fn frame(&self, time: Timestamp) {
		let _ = self.0.send(time);
	}
}

pub fn spawn_frame_driver<S: InputMethodHelper>(node: &Node<InputMethod<S>>) -> FrameDriver {
	let (tx, mut rx) = watch::channel(Timestamp::now());
	let method = Arc::downgrade(node.handler());
	tokio::spawn(async move {
		while rx.changed().await.is_ok() {
			let time = *rx.borrow_and_update();
			let Some(method) = method.upgrade() else {
				break;
			};
			method.send(time).await;
		}
	});
	FrameDriver(tx)
}

/// Order handlers nearest first by whatever `distance` means for this device, and resolve
/// the capture: one that's already active keeps it until it leaves the query, otherwise the
/// nearest requester takes it. A capture is exclusive, so nobody else is in the order.
///
/// `distance` returning None drops that handler from the cycle entirely, which is how a
/// device says "not a candidate" for its own reasons rather than for being far away. A
/// handler already holding a capture keeps it either way, since letting go of one is the
/// query's business and the guard's, not the ordering's.
pub fn order_by_distance<V: Send + Sync + 'static>(
	handlers: &HashMap<QueryableId, CachedHandler<V>>,
	capture_requests: &HashSet<InputHandler>,
	active_capture: Option<&InputHandler>,
	distance: impl Fn(&CachedHandler<V>) -> Option<f32>,
) -> (Vec<InputHandler>, Option<InputHandler>) {
	let mut order: Vec<_> = handlers
		.values()
		.filter(|e| e.spatial.is_some())
		.filter_map(|e| Some((distance(e)?, e.handler.clone())))
		.collect();
	order.sort_by(|(a, _), (b, _)| a.total_cmp(b));

	let capture = active_capture
		.filter(|cap| handlers.values().any(|e| &e.handler == *cap))
		.cloned()
		.or_else(|| {
			order
				.iter()
				.find(|(_, h)| capture_requests.contains(h))
				.map(|(_, h)| h.clone())
		});

	match capture {
		Some(cap) => (vec![cap.clone()], Some(cap)),
		None => (order.into_iter().map(|(_, h)| h).collect(), None),
	}
}

/// Builds the datamap an [`InputMethodHelper`] hands back.
#[derive(Debug, Default, Clone)]
pub struct DatamapBuilder(HashMap<String, DatamapData>);
impl DatamapBuilder {
	pub fn f32(mut self, key: &str, value: f32) -> Self {
		self.0.insert(key.to_string(), DatamapData::Float { value });
		self
	}
	pub fn vec2(mut self, key: &str, value: impl Into<Vec2F>) -> Self {
		self.0.insert(
			key.to_string(),
			DatamapData::Vec2 {
				value: value.into(),
			},
		);
		self
	}
	pub fn build(self) -> HashMap<String, DatamapData> {
		self.0
	}
}

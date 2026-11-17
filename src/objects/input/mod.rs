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
use gluon_ipc::{Context, Handler, Node, NodeError, RefExt};
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
use tokio::sync::watch;
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

	/// the hovered handlers that get input this cycle, closest first
	///
	/// only asked while nothing holds the capture, and the nearest one that requested a
	/// capture takes it. [`order_by_distance`] is what all the built-in devices use
	fn order_handlers(
		&self,
		hovered: &HashMap<QueryableId, Hovered<Self::QueryValue>>,
	) -> impl Future<Output = Vec<QueryableId>> + Send + Sync;

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
pub struct CaptureGuard;
impl InputMethodCaptureHandler for CaptureGuard {}

#[derive(Handler)]
pub struct InputMethod<S: InputMethodHelper> {
	helper: S,
	cache: QueryCache<S::QueryValue>,
	spatial: Arc<Spatial>,
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
		let (node, method) = InputMethodProxy::new_node(Self {
			helper,
			cache,
			spatial,
			tracker: Mutex::default(),
			proxy: OnceLock::new(),
			query: OnceLock::new(),
		})?;
		let proxy = method.into_proxy();
		let _ = node.proxy.set(proxy.clone());
		let _ = node.query.set(Box::new(query));
		Ok((node, proxy))
	}

	pub fn cache(&self) -> &QueryCache<S::QueryValue> {
		&self.cache
	}

	pub fn active_capture_handler_blocking(&self) -> Option<InputHandler> {
		self.cache
			.read_blocking()
			.captured()
			.map(|c| c.target.handler.clone())
	}

	/// force-release the active capture, as if the capturing client dropped its guard
	pub fn stop_active_capture_blocking(&self) {
		self.cache.write_blocking().release();
	}

	async fn grant_capture(
		&self,
		mut request: CaptureRequest,
	) -> Result<Node<CaptureGuard>, NodeError> {
		let (capture, capture_ref) = InputMethodCapture::new_node(CaptureGuard)?;
		// the value should only be consumed here or in Drop
		let sender = request.reply.take().unwrap();
		let sent = sender.send(Some(capture_ref.into_proxy())).is_ok();
		if sent {
			self.helper
				.capture_granted(&request.request_ctx, &request.handler)
				.await;
		}
		Ok(capture)
	}

	/// one dispatch cycle: work out who gets what and send it
	pub async fn send(&self, time: Timestamp) {
		let Some(method) = self.proxy.get() else {
			return;
		};

		// a handler whose get_spatial is still in flight is skipped rather than kept in
		// order, so its first real send is always an input_gained
		let (capture, targets) = {
			let mut handlers = self.cache.write().await;
			handlers.sweep();
			let order = match handlers.captured() {
				Some(_) => Vec::new(),
				None => self.helper.order_handlers(handlers.hovered()).await,
			};
			if let Some((id, request)) = handlers.take_requests(&order)
				&& let Ok(guard) = self
					.grant_capture(request)
					.await
					.inspect_err(|err| error!("failed to grant capture: {err}"))
			{
				handlers.capture(id, guard);
			}
			match handlers.captured() {
				Some(c) => (
					Some(c.target.handler.clone()),
					vec![(
						c.target.handler.clone(),
						c.spatial.clone(),
						c.target.field.clone(),
					)],
				),
				None => (
					None,
					order
						.iter()
						.filter_map(|id| {
							let h = handlers.hovered().get(id)?;
							Some((
								h.target.handler.clone(),
								h.spatial()?.clone(),
								h.target.field.clone(),
							))
						})
						.collect(),
				),
			}
		};

		let (datamap, input) = tokio::join!(self.helper.datamap(), self.helper.input_data(time));

		// an untracked device dispatches to nobody rather than returning early, so whoever was
		// active still gets their input_left
		let mut dispatch: Vec<(InputHandler, SpatialData, SemanticData)> = Vec::new();
		if let Some(input) = input {
			dispatch = targets
				.iter()
				.cloned()
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
			targets
				.iter()
				.map(|(handler, ..)| handler.clone())
				.collect(),
		);

		for (handler, spatial, semantic) in dispatch {
			let sent = if added.contains(&handler) {
				handler.input_gained(method.clone(), time, spatial, semantic)
			} else {
				handler.input_updated(method.clone(), time, spatial, semantic)
			};
			if let Err(err) = sent {
				error!("failed to send input to handler: {err}");
			}
		}
		for handler in removed {
			if let Err(err) = handler.input_left(method.clone(), time)
				&& !matches!(err, gluon_ipc::SendError::Closed)
			{
				error!("failed to send input_left to handler: {err}");
			}
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

pub struct CaptureRequest {
	reply: Option<gluon_ipc::ReplySender<Option<InputMethodCapture>>>,
	handler: InputHandler,
	request_ctx: gluon_ipc::Context,
}
impl Drop for CaptureRequest {
	fn drop(&mut self) {
		if let Some(sender) = self.reply.take() {
			_ = sender.send(None);
		}
	}
}

impl<S: InputMethodHelper> InputMethodHandler for InputMethod<S> {
	async fn request_capture_oneway(
		&self,
		ctx: gluon_ipc::Context,
		handler: InputHandler,
		reply: gluon_ipc::ReplySender<Option<InputMethodCapture>>,
	) -> Result<(), gluon_ipc::SendError> {
		self.cache.write().await.request(CaptureRequest {
			reply: Some(reply),
			handler,
			request_ctx: ctx,
		});
		Ok(())
	}
	async fn request_capture(
		&self,
		_ctx: Context,
		_handler: InputHandler,
	) -> Option<InputMethodCapture> {
		unreachable!()
	}

	async fn get_spatial_data(
		&self,
		_ctx: Context,
		handler: InputHandler,
		time: Timestamp,
	) -> Option<SpatialData> {
		let cached = {
			let handlers = self.cache.read().await;
			match handlers.captured() {
				Some(c) if c.target.handler != handler => return None,
				Some(c) => Some((c.spatial.clone(), c.target.field.clone())),
				None => handlers
					.hovered()
					.values()
					.find(|h| h.target.handler == handler)
					.and_then(|h| Some((h.spatial()?.clone(), h.target.field.clone()))),
			}
		};
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

/// hovered handlers nearest first, by whatever `distance` means for this device
///
/// `distance` returning None drops that handler from the cycle entirely, which is how a
/// device says "not a candidate" for its own reasons rather than for being far away
pub fn order_by_distance<V: Send + Sync + 'static>(
	hovered: &HashMap<QueryableId, Hovered<V>>,
	distance: impl Fn(&Target<V>) -> Option<f32>,
) -> Vec<QueryableId> {
	let mut order: Vec<_> = hovered
		.iter()
		.filter(|(_, h)| h.spatial().is_some())
		.filter_map(|(id, h)| Some((distance(&h.target)?, *id)))
		.collect();
	order.sort_by(|(a, _), (b, _)| a.total_cmp(b));
	order.into_iter().map(|(_, id)| id).collect()
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

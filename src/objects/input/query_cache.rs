use crate::{
	nodes::{ProxyExt as _, fields::Field, spatial::SpatialRef},
	objects::input::{CaptureGuard, CaptureRequest},
};
use gluon_ipc::{Context, Handler, Liveness, Node};
use stardust_xr_protocol::{
	field::{FieldRef as FieldRefProxy, FieldSample, RayMarchResult},
	query::{QueriedInterface, QueryableId},
	spatial::SpatialRef as SpatialRefProxy,
	spatial_query::{BeamQueryHandlerHandler, PointsQueryHandlerHandler},
	suis::InputHandler,
};
use std::{
	collections::{HashMap, hash_map::Entry},
	fmt::{Debug, Formatter},
	sync::Arc,
};
use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

/// what the query knows about an input handler
pub struct Target<V: Send + Sync + 'static> {
	pub handler: InputHandler,
	pub field: Arc<Field>,
	pub value: V,
}
impl<V: Send + Sync + 'static> Target<V> {
	// the handler stays, it's what the dispatch tracker and the spatial are keyed to
	fn refresh(&mut self, new: Self) {
		self.field = new.field;
		self.value = new.value;
	}
}

enum Resolve {
	Pending,
	Ready {
		spatial: Arc<SpatialRef>,
		request: Option<CaptureRequest>,
	},
}

/// a handler the query is currently turning up
pub struct Hovered<V: Send + Sync + 'static> {
	pub target: Target<V>,
	spatial: Resolve,
}
impl<V: Send + Sync + 'static> Hovered<V> {
	/// None while the get_spatial round trip is in flight
	pub fn spatial(&self) -> Option<&Arc<SpatialRef>> {
		match &self.spatial {
			Resolve::Pending => None,
			Resolve::Ready { spatial, .. } => Some(spatial),
		}
	}
}

/// the one handler holding the capture, which keeps it even while the query can't see it
pub struct Captured<V: Send + Sync + 'static> {
	id: QueryableId,
	pub target: Target<V>,
	pub spatial: Arc<SpatialRef>,
	/// the process that requested the capture, None without peer credentials
	pub pid: Option<i32>,
	guard: Node<CaptureGuard>,
	in_query: bool,
}

/// every handler an input method can send to
///
/// a capture owns its handler instead of pointing at one, so there's no capture without a
/// handler and no handler outside the query without a capture
pub struct Handlers<V: Send + Sync + 'static> {
	hovered: HashMap<QueryableId, Hovered<V>>,
	captured: Option<Captured<V>>,
}
impl<V: Send + Sync + 'static> Default for Handlers<V> {
	fn default() -> Self {
		Self {
			hovered: HashMap::new(),
			captured: None,
		}
	}
}
impl<V: Send + Sync + 'static> Handlers<V> {
	pub fn hovered(&self) -> &HashMap<QueryableId, Hovered<V>> {
		&self.hovered
	}
	pub fn captured(&self) -> Option<&Captured<V>> {
		self.captured.as_ref()
	}
	/// everything the query can see right now and that's ready for input
	pub fn in_query(&self) -> impl Iterator<Item = &Target<V>> {
		self.hovered
			.values()
			.filter(|h| h.spatial().is_some())
			.map(|h| &h.target)
			.chain(
				self.captured
					.iter()
					.filter(|c| c.in_query)
					.map(|c| &c.target),
			)
	}

	/// true when the handler is new and its spatial still has to be fetched
	fn enter(&mut self, id: QueryableId, target: Target<V>) -> bool {
		if let Some(c) = self.captured.as_mut().filter(|c| c.id == id) {
			c.target.refresh(target);
			c.in_query = true;
			return false;
		}
		match self.hovered.entry(id) {
			// a beam re-enters every time it flaps across a field boundary, resetting the
			// spatial here would drop the handler out of dispatch for a spurious input_left
			Entry::Occupied(mut e) => {
				e.get_mut().target.refresh(target);
				false
			}
			Entry::Vacant(e) => {
				e.insert(Hovered {
					target,
					spatial: Resolve::Pending,
				});
				true
			}
		}
	}
	fn resolve(&mut self, id: &QueryableId, spatial: Option<Arc<SpatialRef>>) {
		let Some(spatial) = spatial else {
			self.hovered.remove(id);
			return;
		};
		if let Some(Hovered {
			spatial: s @ Resolve::Pending,
			..
		}) = self.hovered.get_mut(id)
		{
			*s = Resolve::Ready {
				spatial,
				request: None,
			};
		}
	}
	fn moved(&mut self, id: &QueryableId, value: V) {
		if let Some(c) = self.captured.as_mut().filter(|c| &c.id == id) {
			c.target.value = value;
		} else if let Some(h) = self.hovered.get_mut(id) {
			h.target.value = value;
		}
	}
	fn left(&mut self, id: &QueryableId) {
		match self.captured.as_mut().filter(|c| &c.id == id) {
			Some(c) => c.in_query = false,
			None => {
				self.hovered.remove(id);
			}
		}
	}

	/// only a hovered handler that's ready can ask, anyone else is told no by the drop
	pub(super) fn request(&mut self, request: CaptureRequest) {
		if let Some(Resolve::Ready { request: slot, .. }) = self
			.hovered
			.values_mut()
			.find(|h| h.target.handler == request.handler)
			.map(|h| &mut h.spatial)
		{
			*slot = Some(request);
		}
	}
	/// the nearest requester in `order` wins, every other pending request is denied
	pub(super) fn take_requests(
		&mut self,
		order: &[QueryableId],
	) -> Option<(QueryableId, CaptureRequest)> {
		let mut requests: HashMap<_, _> = self
			.hovered
			.iter_mut()
			.filter_map(|(id, h)| match &mut h.spatial {
				Resolve::Ready { request, .. } => Some((*id, request.take()?)),
				Resolve::Pending => None,
			})
			.collect();
		order.iter().find_map(|id| requests.remove_entry(id))
	}
	pub(super) fn capture(&mut self, id: QueryableId, guard: Node<CaptureGuard>, pid: Option<i32>) {
		match self.hovered.remove(&id) {
			Some(Hovered {
				target,
				spatial: Resolve::Ready { spatial, .. },
			}) => {
				self.release();
				self.captured = Some(Captured {
					id,
					target,
					spatial,
					pid,
					guard,
					in_query: true,
				});
			}
			Some(h) => {
				self.hovered.insert(id, h);
			}
			None => {}
		}
	}
	/// the handler goes back to being hovered if the query still sees it
	pub(super) fn release(&mut self) {
		let Some(c) = self.captured.take() else {
			return;
		};
		if c.in_query {
			self.hovered.insert(
				c.id,
				Hovered {
					target: c.target,
					spatial: Resolve::Ready {
						spatial: c.spatial,
						request: None,
					},
				},
			);
		}
	}
	/// let go of a capture whose guard the client dropped, and of handlers whose client died
	/// without the query ever saying they left
	pub(super) fn sweep(&mut self) {
		self.captured.take_if(|c| !c.target.handler.alive());
		if self.captured.as_ref().is_some_and(|c| c.guard.is_dead()) {
			self.release();
		}
		self.hovered.retain(|_, h| h.target.handler.alive());
	}
}

/// The input handlers a spatial query is currently turning up, shared between the query
/// adapter and the [`super::InputMethod`] that dispatches to them.
pub struct QueryCache<V: Send + Sync + 'static>(Arc<RwLock<Handlers<V>>>);
impl<V: Send + Sync + 'static> Clone for QueryCache<V> {
	fn clone(&self) -> Self {
		Self(self.0.clone())
	}
}
impl<V: Send + Sync + 'static> Default for QueryCache<V> {
	fn default() -> Self {
		Self(Arc::default())
	}
}
impl<V: Send + Sync + 'static> Debug for QueryCache<V> {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("QueryCache").finish_non_exhaustive()
	}
}
impl<V: Send + Sync + 'static> QueryCache<V> {
	pub async fn read(&self) -> RwLockReadGuard<'_, Handlers<V>> {
		self.0.read().await
	}
	pub fn read_blocking(&self) -> RwLockReadGuard<'_, Handlers<V>> {
		self.0.blocking_read()
	}
	pub(super) async fn write(&self) -> RwLockWriteGuard<'_, Handlers<V>> {
		self.0.write().await
	}
	pub(super) fn write_blocking(&self) -> RwLockWriteGuard<'_, Handlers<V>> {
		self.0.blocking_write()
	}

	async fn entered(
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
		let target = Target {
			handler: handler.clone(),
			field: field.data.clone(),
			value,
		};
		if !self.write().await.enter(obj, target) {
			return;
		}
		let spatial = handler.get_spatial().await.ok().and_then(|s| s.owned());
		self.write().await.resolve(&obj, spatial);
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
		self.0.entered(obj, field, interfaces, march_result).await;
	}
	async fn interfaces_changed(
		&self,
		_ctx: Context,
		_obj: QueryableId,
		_interfaces: Vec<QueriedInterface>,
	) {
	}
	async fn moved(&self, _ctx: Context, obj: QueryableId, march_result: RayMarchResult) {
		self.0.write().await.moved(&obj, march_result);
	}
	async fn left(&self, _ctx: Context, obj: QueryableId) {
		self.0.write().await.left(&obj);
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
		self.0.entered(obj, field, interfaces, sample).await;
	}
	async fn interfaces_changed(
		&self,
		_ctx: Context,
		_obj: QueryableId,
		_interfaces: Vec<QueriedInterface>,
	) {
	}
	async fn moved(&self, _ctx: Context, obj: QueryableId, sample: FieldSample) {
		self.0.write().await.moved(&obj, sample);
	}
	async fn left(&self, _ctx: Context, obj: QueryableId) {
		self.0.write().await.left(&obj);
	}
}

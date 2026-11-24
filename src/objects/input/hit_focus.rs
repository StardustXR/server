use super::spatial_query_interface;
use gluon_ipc::{Context, Handler, Interface, Node, NodeError, RefExt};
use stardust_xr_protocol::{
	field::{FieldRef as FieldRefProxy, FieldSample},
	query::{InterfaceDependency, QueriedInterface, QueryableId},
	spatial::SpatialRef as SpatialRefProxy,
	spatial_query::{
		Point, PointsQuery, PointsQueryHandle, PointsQueryHandler, PointsQueryHandlerHandler,
	},
};
use std::{
	collections::HashMap,
	sync::{Arc, Mutex, OnceLock},
};
use tracing::error;

pub trait Focusable: Interface + RefExt + Clone + Send + Sync + 'static {}
impl<T: Interface + RefExt + Clone + Send + Sync + 'static> Focusable for T {}

/// whatever with interface `I` is closest to some points that follow what an input
/// method is aimed at, like keyboard focus following the mouse
pub struct HitFocus<I: Focusable> {
	matched: Node<Matched<I>>,
	handle: Arc<OnceLock<PointsQueryHandle>>,
}
impl<I: Focusable> HitFocus<I> {
	pub fn new(reference_spatial: SpatialRefProxy) -> Result<Self, NodeError> {
		let (matched, handler) = PointsQueryHandler::new_node(Matched::<I>(Mutex::default()))?;
		let handle = Arc::new(OnceLock::new());
		tokio::spawn({
			let handle = handle.clone();
			async move {
				let result = spatial_query_interface()
					.points_query(PointsQuery {
						handler: handler.into_proxy(),
						interfaces: vec![InterfaceDependency {
							id: I::ID.into(),
							optional: false,
						}],
						reference_spatial,
						points: vec![],
					})
					.await;
				match result {
					Ok(Ok(created)) => {
						let _ = handle.set(created);
					}
					Ok(Err(err)) => error!("failed to create {} focus query: {err}", I::ID),
					Err(err) => error!("failed to create {} focus query: {err}", I::ID),
				}
			}
		});
		Ok(HitFocus { matched, handle })
	}

	pub fn update(&self, points: Vec<Point>) {
		if let Some(handle) = self.handle.get() {
			let _ = handle.update(points);
		}
	}

	pub fn closest(&self) -> Option<I> {
		self.matched
			.0
			.lock()
			.unwrap()
			.values()
			.min_by(|(_, a), (_, b)| a.distance.total_cmp(&b.distance))
			.map(|(i, _)| i.clone())
	}
}

#[derive(Debug, Handler)]
struct Matched<I: Focusable>(Mutex<HashMap<QueryableId, (I, FieldSample)>>);
impl<I: Focusable> PointsQueryHandlerHandler for Matched<I> {
	async fn entered(
		&self,
		_ctx: Context,
		obj: QueryableId,
		_field: FieldRefProxy,
		_spatial: SpatialRefProxy,
		interfaces: Vec<QueriedInterface>,
		sample: FieldSample,
	) {
		let Some(interface) = interfaces.into_iter().find(|i| i.interface_id == I::ID) else {
			return;
		};
		let i = I::from_ref(interface.interface);
		self.0.lock().unwrap().insert(obj, (i, sample));
	}
	async fn interfaces_changed(
		&self,
		_ctx: Context,
		_obj: QueryableId,
		_interfaces: Vec<QueriedInterface>,
	) {
	}
	async fn moved(&self, _ctx: Context, obj: QueryableId, sample: FieldSample) {
		if let Some(entry) = self.0.lock().unwrap().get_mut(&obj) {
			entry.1 = sample;
		}
	}
	async fn left(&self, _ctx: Context, obj: QueryableId) {
		self.0.lock().unwrap().remove(&obj);
	}
}

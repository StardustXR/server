use std::{
	collections::{HashMap, hash_map::Entry},
	fmt::Debug,
	future::Future,
	pin::Pin,
	sync::{Arc, OnceLock, Weak},
};

use glam::Vec3;
use gluon::{Handler, SendError};
use parking_lot::Mutex;
use stardust_xr_protocol::{
	field::FieldRef as FieldRefProxy,
	query::{InterfaceDependency, QueriedInterface, QueryableObjectRef},
	spatial::SpatialRef as SpatialRefProxy,
	spatial_query::{
		BeamQuery, BeamQueryHandler, Point, PointsQuery,
		PointsQueryHandle as PointsQueryHandleProxy, PointsQueryHandleHandler, PointsQueryHandler,
		QueryError, SpatialQueryGuard, SpatialQueryGuardHandler, SpatialQueryInterfaceHandler,
		ZoneQuery, ZoneQueryHandler,
	},
};
use stardust_xr_server_foundation::deduped_string::DedupedStr;

use crate::{
	PION, interface,
	nodes::{
		ProxyExt as _,
		fields::{Field, Ray, ShapeChangedCallback},
		spatial::{MovedCallback, Spatial},
	},
	query::{InterfaceQuery, QUERY_STATE, Queryable, QueryableInterface},
};

/// A single boxed future. Only `AnyQuery::update_interfaces` needs to be both async
/// (it reads the queryable's interface lock) and object-safe, so this is the one place
/// we pay for a heap-allocated future.
type BoxFut<'a> = Pin<Box<dyn Future<Output = ()> + Send + Sync + 'a>>;

/// The per-kind behaviour of a query. Each kind owns its own hit-data type, so a
/// Beam hit can never be handed to a Zone handler — the mismatch is a type error
/// rather than a runtime check. Implementing this trait is the *only* thing a new
/// query kind has to do: discovery, the entered/moved/left state machine, interface
/// tracking, and callback wiring are all provided by [`Query`].
trait QueryKind: Send + Sync + Debug + 'static {
	/// Kind-specific evidence that a queryable currently matches. Only obtainable
	/// from [`QueryKind::hit`], and consumed by [`entered`](QueryKind::entered) /
	/// [`moved`](QueryKind::moved), so those events cannot be emitted without a hit.
	type Hit;

	/// The spatial whose movement re-evaluates the whole query, plus an optional
	/// field whose shape change does the same (Zone). Wired once by [`register_query`].
	fn anchors(&self) -> (&Arc<Spatial>, Option<&Arc<Field>>);

	/// Geometric test for a single queryable. `None` means it does not match.
	fn hit(&self, queryable: &Queryable) -> Option<Self::Hit>;

	fn entered(
		&self,
		queryable: &Queryable,
		interfaces: Vec<QueriedInterface>,
		hit: Self::Hit,
	) -> Result<(), SendError>;
	fn moved(&self, queryable: &Queryable, hit: Self::Hit) -> Result<(), SendError>;
	fn interfaces_changed(
		&self,
		obj: QueryableObjectRef,
		interfaces: Vec<QueriedInterface>,
	) -> Result<(), SendError>;
	fn left(&self, obj: QueryableObjectRef) -> Result<(), SendError>;
}

/// Per-(query, queryable) tracking state — the single source of truth. A queryable is
/// *interested in* this query iff it has an entry here (it satisfies every required
/// interface); it is *matched* iff `matched` is true. `matched` is only ever flipped
/// inside [`Query::reconcile`], in lockstep with the emitted event, so state and
/// notifications cannot disagree.
#[derive(Debug)]
struct Tracked {
	queryable: Weak<Queryable>,
	/// Last-reported set of matching interfaces, in query dependency order. Stored as
	/// the wire form (not `Arc<QueryableInterface>`) so it does not keep the
	/// queryable's interface alive — that's what lets a dropped interface guard be
	/// observed as the interface going missing.
	interfaces: Vec<QueriedInterface>,
	matched: bool,
	_move: MovedCallback,
	_shape: ShapeChangedCallback,
}

#[derive(Debug)]
struct Query<K: QueryKind> {
	interfaces: Vec<InterfaceQuery>,
	tracked: Mutex<HashMap<u64, Tracked>>,
	/// Keeps the query's own anchor callbacks (move + optional shape) alive.
	self_callbacks: OnceLock<(MovedCallback, Option<ShapeChangedCallback>)>,
	kind: K,
}

/// Object-safe facade over `Query<K>` so the global registry can hold mixed kinds.
/// `K::Hit` never escapes `Query<K>`'s own methods, keeping this trait object-safe.
pub(super) trait AnyQuery: Send + Sync {
	/// Re-sync interest + interfaces for one queryable, then re-evaluate the hit.
	fn update_interfaces(self: Arc<Self>, queryable: Arc<Queryable>) -> BoxFut<'static>;
	/// A queryable is going away: drop its tracking and fire `left` if it was matched.
	fn queryable_destroyed(self: Arc<Self>, queryable: &Queryable);
}
impl<K: QueryKind> AnyQuery for Query<K> {
	fn update_interfaces(self: Arc<Self>, queryable: Arc<Queryable>) -> BoxFut<'static> {
		Box::pin(async move { self.update_interfaces_impl(&queryable).await })
	}
	fn queryable_destroyed(self: Arc<Self>, queryable: &Queryable) {
		let removed = self.tracked.lock().remove(&queryable.id);
		if let Some(tracked) = removed
			&& tracked.matched
		{
			_ = self.kind.left(queryable.obj_ref());
		}
	}
}

impl<K: QueryKind> Query<K> {
	/// Re-test one queryable's geometry and emit the resulting transition. This is the
	/// only place `Tracked::matched` changes. `hit` is synchronous, so the whole
	/// transition runs under one short lock with no `.await` held across it.
	fn reconcile(&self, queryable: &Arc<Queryable>) {
		let mut tracked = self.tracked.lock();
		let Some(entry) = tracked.get_mut(&queryable.id) else {
			return;
		};
		match (self.kind.hit(queryable), entry.matched) {
			(Some(hit), false) => {
				let interfaces = entry.interfaces.clone();
				entry.matched = true;
				_ = self.kind.entered(queryable, interfaces, hit);
			}
			(Some(hit), true) => {
				_ = self.kind.moved(queryable, hit);
			}
			(None, true) => {
				entry.matched = false;
				_ = self.kind.left(queryable.obj_ref());
			}
			(None, false) => {}
		}
	}

	/// Bring one queryable's interest + interface set up to date, then re-evaluate it.
	async fn update_interfaces_impl(self: &Arc<Self>, queryable: &Arc<Queryable>) {
		let have: HashMap<Arc<DedupedStr>, Arc<QueryableInterface>> = queryable
			.interfaces
			.read()
			.await
			.get_valid_contents()
			.into_iter()
			.map(|i| (i.interface_id.clone(), i))
			.collect();

		let Some(matched) = compute_interfaces(&self.interfaces, &have) else {
			// No longer interested (a required interface went missing): stop tracking
			// it, and tell the client it left if it had been matched.
			let removed = self.tracked.lock().remove(&queryable.id);
			if let Some(tracked) = removed
				&& tracked.matched
			{
				_ = self.kind.left(queryable.obj_ref());
			}
			return;
		};

		let interfaces = proto_interfaces(&matched);
		{
			let mut tracked = self.tracked.lock();
			match tracked.entry(queryable.id) {
				Entry::Occupied(mut occupied) => {
					let entry = occupied.get_mut();
					if entry.interfaces != interfaces {
						entry.interfaces = interfaces.clone();
						_ = self.kind.interfaces_changed(queryable.obj_ref(), interfaces);
					}
				}
				Entry::Vacant(slot) => {
					slot.insert(Tracked {
						queryable: Arc::downgrade(queryable),
						interfaces,
						matched: false,
						_move: self.watch_queryable_moved(queryable),
						_shape: self.watch_queryable_shape(queryable),
					});
				}
			}
		}

		self.reconcile(queryable);
	}

	/// Re-evaluate every tracked queryable — used when the query's own anchor moves
	/// or its zone field reshapes.
	fn self_moved(&self) {
		let queryables: Vec<Arc<Queryable>> = self
			.tracked
			.lock()
			.values()
			.filter_map(|tracked| tracked.queryable.upgrade())
			.collect();
		for queryable in queryables {
			self.reconcile(&queryable);
		}
	}

	/// Wire the query's own anchor callbacks and back-fill against the queryables that
	/// already exist. Both first contact and later updates go through
	/// `update_interfaces_impl`, so they cannot diverge.
	async fn init(self: &Arc<Self>) {
		let (anchor_spatial, anchor_field) = self.kind.anchors();
		let moved = anchor_spatial.moved_callback(self.self_moved_closure());
		let shape = anchor_field.map(|field| field.shape_changed_callback(self.self_moved_closure()));
		_ = self.self_callbacks.set((moved, shape));

		for queryable in QUERY_STATE.all_queryables.get_valid_contents() {
			self.update_interfaces_impl(&queryable).await;
		}
	}

	fn watch_queryable_moved(self: &Arc<Self>, queryable: &Arc<Queryable>) -> MovedCallback {
		queryable
			.field
			.data
			.spatial
			.moved_callback(self.requery_closure(queryable))
	}
	fn watch_queryable_shape(self: &Arc<Self>, queryable: &Arc<Queryable>) -> ShapeChangedCallback {
		queryable
			.field
			.data
			.shape_changed_callback(self.requery_closure(queryable))
	}
	/// Closure that re-tests a single queryable against this query when that queryable
	/// moves or reshapes. Deferred onto the runtime so it never runs while the mover
	/// holds a spatial lock.
	fn requery_closure(
		self: &Arc<Self>,
		queryable: &Arc<Queryable>,
	) -> impl Fn() + Send + Sync + 'static {
		let query = Arc::downgrade(self);
		let queryable = Arc::downgrade(queryable);
		move || {
			if let Some(query) = query.upgrade()
				&& let Some(queryable) = queryable.upgrade()
			{
				tokio::spawn(async move { query.reconcile(&queryable) });
			}
		}
	}
	/// Closure that re-evaluates the whole query when its own anchor moves/reshapes.
	fn self_moved_closure(self: &Arc<Self>) -> impl Fn() + Send + Sync + 'static {
		let query = Arc::downgrade(self);
		move || {
			if let Some(query) = query.upgrade() {
				tokio::spawn(async move { query.self_moved() });
			}
		}
	}
}
impl<K: QueryKind> Drop for Query<K> {
	fn drop(&mut self) {
		let this: &dyn AnyQuery = self;
		QUERY_STATE.queries.remove(this);
	}
}

/// Resolve a query's interface dependencies against the interfaces a queryable
/// actually has. Returns the matching set (required + present optionals, in
/// dependency order), or `None` if a *required* interface is missing — the single
/// place that decision is made.
fn compute_interfaces(
	deps: &[InterfaceQuery],
	have: &HashMap<Arc<DedupedStr>, Arc<QueryableInterface>>,
) -> Option<Vec<Arc<QueryableInterface>>> {
	let mut out = Vec::new();
	for dep in deps {
		match have.get(&dep.id) {
			Some(interface) => out.push(interface.clone()),
			None if dep.optional => {}
			None => return None,
		}
	}
	Some(out)
}

fn proto_interfaces(interfaces: &[Arc<QueryableInterface>]) -> Vec<QueriedInterface> {
	interfaces
		.iter()
		.map(|interface| QueriedInterface {
			interface_id: interface.interface_id.get_string().clone(),
			interface: interface.interface_ref.clone(),
		})
		.collect()
}

/// Parse protocol interface dependencies, enforcing that at least one is required.
async fn parse_interfaces(
	deps: Vec<InterfaceDependency>,
) -> Result<Vec<InterfaceQuery>, QueryError> {
	let mut interfaces = Vec::with_capacity(deps.len());
	let mut found_required = false;
	for dep in deps {
		found_required |= !dep.optional;
		interfaces.push(InterfaceQuery {
			id: DedupedStr::get(dep.id).await,
			optional: dep.optional,
		});
	}
	if !found_required {
		return Err(QueryError::NoRequiredInterfaces);
	}
	Ok(interfaces)
}

/// Build a query of the given kind, register it globally, wire its anchor callbacks,
/// and back-fill against existing queryables — the one place this ritual lives, so a
/// new query kind can't skip a step.
async fn register_query<K: QueryKind>(
	kind: K,
	deps: Vec<InterfaceDependency>,
) -> Result<Arc<Query<K>>, QueryError> {
	let interfaces = parse_interfaces(deps).await?;
	let query = Arc::new(Query {
		interfaces,
		tracked: Mutex::new(HashMap::new()),
		self_callbacks: OnceLock::new(),
		kind,
	});
	let dyn_query: Arc<dyn AnyQuery> = query.clone();
	QUERY_STATE.queries.add_raw(&dyn_query);
	query.init().await;
	Ok(query)
}

// === query kinds ===

#[derive(Debug)]
struct BeamKind {
	handler: BeamQueryHandler,
	ref_space: Arc<Spatial>,
	origin: Vec3,
	dir: Vec3,
	max_length: f32,
}
struct BeamHit {
	deepest_point_distance: f32,
	distance: f32,
}
impl QueryKind for BeamKind {
	type Hit = BeamHit;
	fn anchors(&self) -> (&Arc<Spatial>, Option<&Arc<Field>>) {
		(&self.ref_space, None)
	}
	fn hit(&self, queryable: &Queryable) -> Option<BeamHit> {
		if !queryable.spatial.visible() {
			return None;
		}
		if !queryable.field.data.spatial.visible() {
			return None;
		}
		let ray_march = queryable.field.data.ray_march(Ray {
			origin: self.origin,
			direction: self.dir,
			space: self.ref_space.clone(),
		});
		(ray_march.min_distance <= 0.0 && ray_march.deepest_point_distance <= self.max_length)
			.then_some(BeamHit {
				deepest_point_distance: ray_march.deepest_point_distance,
				distance: ray_march.min_distance,
			})
	}
	fn entered(
		&self,
		queryable: &Queryable,
		interfaces: Vec<QueriedInterface>,
		hit: BeamHit,
	) -> Result<(), SendError> {
		self.handler.intersected(
			queryable.obj_ref(),
			queryable.field_ref(),
			queryable.spatial_ref(),
			interfaces,
			hit.deepest_point_distance,
			hit.distance,
		)
	}
	fn moved(&self, queryable: &Queryable, hit: BeamHit) -> Result<(), SendError> {
		self.handler
			.moved(queryable.obj_ref(), hit.deepest_point_distance, hit.distance)
	}
	fn interfaces_changed(
		&self,
		obj: QueryableObjectRef,
		interfaces: Vec<QueriedInterface>,
	) -> Result<(), SendError> {
		self.handler.interfaces_changed(obj, interfaces)
	}
	fn left(&self, obj: QueryableObjectRef) -> Result<(), SendError> {
		self.handler.left(obj)
	}
}

#[derive(Debug)]
struct ZoneKind {
	handler: ZoneQueryHandler,
	field: Arc<Field>,
	margin: f32,
}
struct ZoneHit {
	pos: Vec3,
	distance: f32,
}
impl QueryKind for ZoneKind {
	type Hit = ZoneHit;
	fn anchors(&self) -> (&Arc<Spatial>, Option<&Arc<Field>>) {
		(&self.field.spatial, Some(&self.field))
	}
	fn hit(&self, queryable: &Queryable) -> Option<ZoneHit> {
		if !queryable.spatial.visible() {
			return None;
		}
		if !self.field.spatial.visible() {
			return None;
		}
		let (_scale, _rotation, pos) =
			Spatial::space_to_space_matrix(Some(&queryable.spatial), Some(&self.field.spatial))
				.to_scale_rotation_translation();
		let distance = self.field.local_sample(pos.into()).distance;
		(distance < self.margin).then_some(ZoneHit { pos, distance })
	}
	fn entered(
		&self,
		queryable: &Queryable,
		interfaces: Vec<QueriedInterface>,
		hit: ZoneHit,
	) -> Result<(), SendError> {
		self.handler.entered(
			queryable.obj_ref(),
			queryable.field_ref(),
			queryable.spatial_ref(),
			interfaces,
			hit.pos.into(),
			hit.distance,
		)
	}
	fn moved(&self, queryable: &Queryable, hit: ZoneHit) -> Result<(), SendError> {
		self.handler
			.moved(queryable.obj_ref(), hit.pos.into(), hit.distance)
	}
	fn interfaces_changed(
		&self,
		obj: QueryableObjectRef,
		interfaces: Vec<QueriedInterface>,
	) -> Result<(), SendError> {
		self.handler.interfaces_changed(obj, interfaces)
	}
	fn left(&self, obj: QueryableObjectRef) -> Result<(), SendError> {
		self.handler.left(obj)
	}
}

#[derive(Debug)]
struct PointsKind {
	handler: PointsQueryHandler,
	ref_space: Arc<Spatial>,
	points: Mutex<Vec<Point>>,
}
struct PointsHit {
	distance: f32,
}
impl QueryKind for PointsKind {
	type Hit = PointsHit;
	fn anchors(&self) -> (&Arc<Spatial>, Option<&Arc<Field>>) {
		(&self.ref_space, None)
	}
	fn hit(&self, queryable: &Queryable) -> Option<PointsHit> {
		self.points
			.lock()
			.iter()
			.map(|p| {
				let distance = queryable
					.field
					.data
					.sample(&self.ref_space, p.point.into())
					.distance;
				(distance - p.margin, distance)
			})
			.reduce(|(sort1, distance1), (sort2, distance2)| {
				if sort1 < sort2 {
					(sort1, distance1)
				} else {
					(sort2, distance2)
				}
			})
			.filter(|(sort, _)| *sort < 0.0)
			.map(|(_, distance)| PointsHit { distance })
	}
	fn entered(
		&self,
		queryable: &Queryable,
		interfaces: Vec<QueriedInterface>,
		hit: PointsHit,
	) -> Result<(), SendError> {
		self.handler.entered(
			queryable.obj_ref(),
			queryable.field_ref(),
			queryable.spatial_ref(),
			interfaces,
			hit.distance,
		)
	}
	fn moved(&self, queryable: &Queryable, hit: PointsHit) -> Result<(), SendError> {
		self.handler.moved(queryable.obj_ref(), hit.distance)
	}
	fn interfaces_changed(
		&self,
		obj: QueryableObjectRef,
		interfaces: Vec<QueriedInterface>,
	) -> Result<(), SendError> {
		self.handler.interfaces_changed(obj, interfaces)
	}
	fn left(&self, obj: QueryableObjectRef) -> Result<(), SendError> {
		self.handler.left(obj)
	}
}

/// Proxy-building helpers shared by every query kind.
impl Queryable {
	fn obj_ref(&self) -> QueryableObjectRef {
		QueryableObjectRef::from_handler(&self.queryable_ref)
	}
	fn field_ref(&self) -> FieldRefProxy {
		FieldRefProxy::from_handler(self.field.get_ref())
	}
	fn spatial_ref(&self) -> SpatialRefProxy {
		SpatialRefProxy::from_handler(self.spatial.get_ref())
	}
}

// === protocol surface ===

interface!(SpatialQueryInterface);
impl SpatialQueryInterfaceHandler for SpatialQueryInterface {
	async fn beam_query(
		&self,
		_ctx: gluon::Context,
		query: BeamQuery,
	) -> Result<SpatialQueryGuard, QueryError> {
		let BeamQuery {
			handler,
			interfaces,
			reference_spatial,
			direction,
			origin,
			max_length,
		} = query;
		let ref_space = reference_spatial.owned().ok_or(QueryError::InvalidRef)?;
		let query = register_query(
			BeamKind {
				handler,
				ref_space: (**ref_space).clone(),
				origin: origin.into(),
				dir: direction.into(),
				max_length,
			},
			interfaces,
		)
		.await?;
		let guard = PION.register_object(Guard(query)).to_service();
		Ok(SpatialQueryGuard::from_handler(&guard))
	}

	async fn zone_query(
		&self,
		_ctx: gluon::Context,
		query: ZoneQuery,
	) -> Result<SpatialQueryGuard, QueryError> {
		let ZoneQuery {
			handler,
			interfaces,
			zone_field,
			margin,
		} = query;
		let field = zone_field.owned().ok_or(QueryError::InvalidRef)?;
		let query = register_query(
			ZoneKind {
				handler,
				field: field.data.clone(),
				margin,
			},
			interfaces,
		)
		.await?;
		let guard = PION.register_object(Guard(query)).to_service();
		Ok(SpatialQueryGuard::from_handler(&guard))
	}

	async fn points_query(
		&self,
		_ctx: gluon::Context,
		query: PointsQuery,
	) -> Result<PointsQueryHandleProxy, QueryError> {
		let PointsQuery {
			handler,
			interfaces,
			reference_spatial,
			points,
		} = query;
		let ref_space = reference_spatial.owned().ok_or(QueryError::InvalidRef)?;
		let query = register_query(
			PointsKind {
				handler,
				ref_space: (**ref_space).clone(),
				points: Mutex::new(points),
			},
			interfaces,
		)
		.await?;
		let handle = PION.register_object(PointsQueryHandle(query)).to_service();
		Ok(PointsQueryHandleProxy::from_handler(&handle))
	}
}

#[derive(Debug, Handler)]
struct PointsQueryHandle(Arc<Query<PointsKind>>);
impl PointsQueryHandleHandler for PointsQueryHandle {
	async fn update_points(&self, _ctx: gluon::Context, points: Vec<Point>) {
		*self.0.kind.points.lock() = points;
		self.0.self_moved();
	}
}

#[expect(unused)]
#[derive(Debug, Handler)]
struct Guard<K: QueryKind>(Arc<Query<K>>);
impl<K: QueryKind> SpatialQueryGuardHandler for Guard<K> {}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::nodes::{fields::Field, spatial::Spatial};
	use glam::{Mat4, Vec3};
	use stardust_xr_protocol::{field::Shape, spatial_query::Point};
	use std::sync::Arc;

	fn make_spatial(x: f32, y: f32, z: f32) -> Arc<Spatial> {
		Spatial::test_new(None, Mat4::from_translation(Vec3::new(x, y, z)))
	}

	fn make_field(spatial: Arc<Spatial>, shape: Shape) -> Arc<Field> {
		Arc::new(Field::test_new(spatial, shape))
	}

	// Mirrors ZoneKind::hit() math (sans visibility checks).
	fn zone_check(queryable_spatial: &Spatial, zone_field: &Field, margin: f32) -> bool {
		let (_s, _r, pos) =
			Spatial::space_to_space_matrix(Some(queryable_spatial), Some(&zone_field.spatial))
				.to_scale_rotation_translation();
		let distance = zone_field.local_sample(pos.into()).distance;
		distance < margin
	}

	// Mirrors BeamKind::hit() math.
	fn beam_check(
		target_field: &Arc<Field>,
		ref_space: &Arc<Spatial>,
		origin: Vec3,
		dir: Vec3,
		max_length: f32,
	) -> bool {
		let result = target_field.ray_march(Ray {
			origin,
			direction: dir,
			space: ref_space.clone(),
		});
		result.min_distance <= 0.0 && result.deepest_point_distance <= max_length
	}

	// Mirrors PointsKind::hit() math.
	fn points_check(target_field: &Arc<Field>, ref_space: &Arc<Spatial>, points: &[Point]) -> bool {
		let best = points
			.iter()
			.map(|p| {
				let d = target_field.sample(ref_space, p.point.into()).distance;
				(d - p.margin, d)
			})
			.reduce(|(s1, d1), (s2, d2)| if s1 < s2 { (s1, d1) } else { (s2, d2) });
		best.is_some_and(|(d, _)| d < 0.0)
	}

	// --- zone ---

	#[test]
	fn zone_object_inside_sphere_hits() {
		let zone = make_field(
			Spatial::test_new(None, Mat4::IDENTITY),
			Shape::Sphere { radius: 1.0 },
		);
		let queryable = make_spatial(0.0, 0.0, 0.5);
		assert!(zone_check(&queryable, &zone, 0.0));
	}

	#[test]
	fn zone_object_outside_sphere_misses() {
		let zone = make_field(
			Spatial::test_new(None, Mat4::IDENTITY),
			Shape::Sphere { radius: 1.0 },
		);
		let queryable = make_spatial(2.0, 0.0, 0.0);
		assert!(!zone_check(&queryable, &zone, 0.0));
	}

	#[test]
	fn zone_margin_extends_detection_range() {
		let zone = make_field(
			Spatial::test_new(None, Mat4::IDENTITY),
			Shape::Sphere { radius: 1.0 },
		);
		// 0.5 m outside the sphere surface — within margin 1.0
		let queryable = make_spatial(1.5, 0.0, 0.0);
		assert!(zone_check(&queryable, &zone, 1.0));
	}

	#[test]
	fn zone_object_beyond_margin_misses() {
		let zone = make_field(
			Spatial::test_new(None, Mat4::IDENTITY),
			Shape::Sphere { radius: 1.0 },
		);
		// 1.5 m outside the sphere surface, margin only 1.0
		let queryable = make_spatial(2.5, 0.0, 0.0);
		assert!(!zone_check(&queryable, &zone, 1.0));
	}

	#[test]
	fn zone_field_offset_from_origin() {
		// Zone at (3,0,0), queryable at (3,0,0.5) — should be inside
		let zone = make_field(make_spatial(3.0, 0.0, 0.0), Shape::Sphere { radius: 1.0 });
		let queryable = make_spatial(3.0, 0.0, 0.5);
		assert!(zone_check(&queryable, &zone, 0.0));
	}

	// --- beam ---

	#[test]
	fn beam_hits_sphere_head_on() {
		let ref_space = Spatial::test_new(None, Mat4::IDENTITY);
		let sphere = make_field(
			Spatial::test_new(None, Mat4::IDENTITY),
			Shape::Sphere { radius: 1.0 },
		);
		// Beam from (-3,0,0) along +X — passes through sphere at origin
		assert!(beam_check(
			&sphere,
			&ref_space,
			Vec3::new(-3.0, 0.0, 0.0),
			Vec3::X,
			f32::MAX
		));
	}

	#[test]
	fn beam_misses_sphere_when_offset() {
		let ref_space = Spatial::test_new(None, Mat4::IDENTITY);
		let sphere = make_field(
			Spatial::test_new(None, Mat4::IDENTITY),
			Shape::Sphere { radius: 1.0 },
		);
		// Beam offset 2 m on Y — clears the sphere entirely
		assert!(!beam_check(
			&sphere,
			&ref_space,
			Vec3::new(-3.0, 2.0, 0.0),
			Vec3::X,
			f32::MAX
		));
	}

	#[test]
	fn beam_max_length_excludes_far_sphere() {
		let ref_space = Spatial::test_new(None, Mat4::IDENTITY);
		// Sphere 10 m away along +X
		let sphere = make_field(make_spatial(10.0, 0.0, 0.0), Shape::Sphere { radius: 1.0 });
		// max_length = 5.0 — beam stops before reaching the sphere
		assert!(!beam_check(&sphere, &ref_space, Vec3::ZERO, Vec3::X, 5.0));
	}

	#[test]
	fn beam_hits_sphere_within_max_length() {
		let ref_space = Spatial::test_new(None, Mat4::IDENTITY);
		let sphere = make_field(make_spatial(3.0, 0.0, 0.0), Shape::Sphere { radius: 1.0 });
		// Sphere near face at 2 m, well within max_length = 10.0
		assert!(beam_check(&sphere, &ref_space, Vec3::ZERO, Vec3::X, 10.0));
	}

	// --- points ---

	#[test]
	fn points_single_point_inside_sphere_hits() {
		let ref_space = Spatial::test_new(None, Mat4::IDENTITY);
		let sphere = make_field(
			Spatial::test_new(None, Mat4::IDENTITY),
			Shape::Sphere { radius: 1.0 },
		);
		let pts = vec![Point {
			point: Vec3::ZERO.into(),
			margin: 0.0,
		}];
		assert!(points_check(&sphere, &ref_space, &pts));
	}

	#[test]
	fn points_single_point_outside_sphere_misses() {
		let ref_space = Spatial::test_new(None, Mat4::IDENTITY);
		let sphere = make_field(
			Spatial::test_new(None, Mat4::IDENTITY),
			Shape::Sphere { radius: 1.0 },
		);
		let pts = vec![Point {
			point: Vec3::new(3.0, 0.0, 0.0).into(),
			margin: 0.0,
		}];
		assert!(!points_check(&sphere, &ref_space, &pts));
	}

	#[test]
	fn points_margin_extends_detection_range() {
		let ref_space = Spatial::test_new(None, Mat4::IDENTITY);
		let sphere = make_field(
			Spatial::test_new(None, Mat4::IDENTITY),
			Shape::Sphere { radius: 1.0 },
		);
		// Point 0.5 m outside sphere surface; margin = 1.0 → hit
		let pts = vec![Point {
			point: Vec3::new(1.5, 0.0, 0.0).into(),
			margin: 1.0,
		}];
		assert!(points_check(&sphere, &ref_space, &pts));
	}

	#[test]
	fn points_any_inside_causes_hit() {
		let ref_space = Spatial::test_new(None, Mat4::IDENTITY);
		let sphere = make_field(
			Spatial::test_new(None, Mat4::IDENTITY),
			Shape::Sphere { radius: 1.0 },
		);
		let pts = vec![
			Point {
				point: Vec3::new(5.0, 0.0, 0.0).into(),
				margin: 0.0,
			},
			Point {
				point: Vec3::ZERO.into(),
				margin: 0.0,
			}, // inside
			Point {
				point: Vec3::new(-5.0, 0.0, 0.0).into(),
				margin: 0.0,
			},
		];
		assert!(points_check(&sphere, &ref_space, &pts));
	}

	#[test]
	fn points_all_outside_misses() {
		let ref_space = Spatial::test_new(None, Mat4::IDENTITY);
		let sphere = make_field(
			Spatial::test_new(None, Mat4::IDENTITY),
			Shape::Sphere { radius: 1.0 },
		);
		let pts = vec![
			Point {
				point: Vec3::new(5.0, 0.0, 0.0).into(),
				margin: 0.0,
			},
			Point {
				point: Vec3::new(-5.0, 0.0, 0.0).into(),
				margin: 0.0,
			},
		];
		assert!(!points_check(&sphere, &ref_space, &pts));
	}
}

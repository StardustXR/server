use std::{
	collections::HashMap,
	hash::{BuildHasher, Hash, Hasher, RandomState},
	sync::{Arc, OnceLock, Weak},
};

use bevy::prelude::Deref;
use glam::Vec3;
use gluon::Handler;
use stardust_xr_protocol::{
	field::FieldRef as FieldRefProxy,
	query::{QueriedInterface, QueryableObjectRef},
	spatial::SpatialRef as SpatialRefProxy,
	spatial_query::{
		BeamQuery, BeamQueryHandler, Point, PointsQuery, PointsQueryHandleHandler,
		PointsQueryHandler, SpatialQueryGuard, SpatialQueryGuardHandler,
		SpatialQueryInterfaceHandler, ZoneQuery, ZoneQueryHandler,
	},
};
use stardust_xr_server_foundation::{
	deduped_string::DedupedStr,
	registry::{OwnedRegistry, Registry},
};
use tokio::sync::RwLock;
use tracing::{error, info};

use crate::{
	PION, interface,
	nodes::{
		ProxyExt as _,
		fields::{Field, Ray, ShapeChangedCallback},
		spatial::{MovedCallback, Spatial},
	},
	query::{InterfaceQuery, QUERY_STATE, Queryable, QueryableInterface},
};

// state changes
// - [x] interfaces added to object
// - [x] interfaces dropped from object
// - [x] object dropped
// - [x] query created
// - [x] query dropped
// - [x] query spatial moved
// - [x] query field shape changes
// - [x] object field moved
// - [x] object field shape changes
// - [x] register moved handler when object becomes a valid query target
// - [x] drop moved handler when object becomees an invalid query target

#[derive(Debug)]
struct QueryableInterest {
	interfaces: Registry<QueryableInterface>,
	_move_handle: MovedCallback,
	_shape_callback: ShapeChangedCallback,
}
#[derive(Debug)]
pub(super) struct Query {
	interfaces: Vec<InterfaceQuery>,
	interesting_queryables: RwLock<HashMap<WeakPtrHash<Queryable>, QueryableInterest>>,
	matching_queryables: Registry<Queryable>,
	self_moved_handle: OnceLock<MovedCallback>,
	inner: QueryType,
}
#[derive(Debug)]
enum QueryType {
	Zone {
		handler: ZoneQueryHandler,
		field: Arc<Field>,
		margin: f32,
		_shape_changed: OnceLock<ShapeChangedCallback>,
	},
	Beam {
		handler: BeamQueryHandler,
		ref_space: Arc<Spatial>,
		origin: Vec3,
		dir: Vec3,
		max_length: f32,
	},
	Points {
		handler: PointsQueryHandler,
		ref_space: Arc<Spatial>,
		points: RwLock<Vec<Point>>,
	},
}
impl Query {
	pub fn queryable_destroyed(self: Arc<Self>, queryable: &Queryable) {
		if self.matching_queryables.contains(queryable) {
			self.matching_queryables.remove(queryable);
			_ = self
				.inner
				.left(QueryableObjectRef::from_handler(&queryable.queryable_ref));
		}
		let queryable_addr = (queryable) as *const _ as usize;
		tokio::spawn(async move {
			self.interesting_queryables
				.write()
				.await
				.retain(|k, _| k.as_ptr().addr() != queryable_addr);
		});
	}
	pub async fn update_interfaces(self: &Arc<Self>, queryable: &Arc<Queryable>) {
		let v = queryable
			.interfaces
			.read()
			.await
			.get_valid_contents()
			.into_iter()
			.map(|v| (v.interface_id.clone(), v))
			.collect::<HashMap<_, _>>();
		let queryable_key = WeakPtrHash(Arc::downgrade(queryable));
		let state = RandomState::new();

		let hash = if let Some(v) = self.interesting_queryables.read().await.get(&queryable_key) {
			let mut hasher = state.build_hasher();
			v.interfaces
				.get_valid_contents()
				.iter()
				.for_each(|v| v.interface_id.hash(&mut hasher));
			v.interfaces.clear();
			Some(hasher.finish())
		} else {
			None
		};
		for i in self.interfaces.iter() {
			if !v.contains_key(&i.id) && !i.optional {
				if self.matching_queryables.contains(queryable) {
					self.matching_queryables.remove(queryable);
					_ = self
						.inner
						.left(QueryableObjectRef::from_handler(&queryable.queryable_ref));
				}
				self.interesting_queryables
					.write()
					.await
					.remove(&queryable_key);
				return;
			}
			if let Some(v) = v.get(&i.id) {
				self.interesting_queryables
					.write()
					.await
					.entry(queryable_key.clone())
					.or_insert_with(|| QueryableInterest {
						interfaces: Registry::new(),
						_move_handle: queryable.field.data.spatial.moved_callback({
							let query = Arc::downgrade(self);
							let queryable = Arc::downgrade(queryable);
							move || {
								if let Some(query) = query.upgrade()
									&& let Some(queryable) = queryable.upgrade()
								{
									tokio::spawn(async move {
										query.update_hit_queryable(&queryable).await;
									});
								}
							}
						}),
						_shape_callback: queryable.field.data.shape_changed_callback({
							let query = Arc::downgrade(self);
							let queryable = Arc::downgrade(queryable);
							move || {
								if let Some(query) = query.upgrade()
									&& let Some(queryable) = queryable.upgrade()
								{
									tokio::spawn(async move {
										query.update_hit_queryable(&queryable).await;
									});
								}
							}
						}),
					})
					.interfaces
					.add_raw(v);
			}
		}
		let new_hash = if let Some(v) = self.interesting_queryables.read().await.get(&queryable_key)
		{
			let mut hasher = state.build_hasher();
			v.interfaces
				.get_valid_contents()
				.iter()
				.for_each(|v| v.interface_id.hash(&mut hasher));
			Some(hasher.finish())
		} else {
			error!("somehow reached second hash point without being interested in the queryable");
			None
		};
		if hash.is_some_and(|hash| Some(hash) != new_hash) {
			let interfaces = self
				.interesting_queryables
				.read()
				.await
				.get(&queryable_key)
				.unwrap()
				.interfaces
				.get_valid_contents()
				.into_iter()
				.map(|v| QueriedInterface {
					interface_id: v.interface_id.get_string().clone(),
					interface: v.interface_ref.clone(),
				})
				.collect();
			_ = self.inner.interfaces_changed(
				QueryableObjectRef::from_handler(&queryable.queryable_ref),
				interfaces,
			);
		}
	}
	pub(super) async fn update_hit_queryable(&self, queryable: &Arc<Queryable>) {
		let interfaces_guard = self.interesting_queryables.read().await;
		let Some(interfaces) = interfaces_guard.get(&WeakPtrHash(Arc::downgrade(queryable))) else {
			return;
		};
		let r = self.inner.hit(queryable).await;
		match (r, self.matching_queryables.contains(queryable)) {
			(None, true) => {
				info!("removing queryable");
				self.matching_queryables.remove(queryable);
				info!("removed queryable");
				_ = self
					.inner
					.left(QueryableObjectRef::from_handler(&queryable.queryable_ref));
			}
			(Some(v), true) => {
				_ = self.inner.moved(queryable, v);
			}
			(Some(v), false) => {
				info!("inserting queryable");
				self.matching_queryables.add_raw(queryable);
				info!("inserted queryable");
				_ = self
					.inner
					.match_gained(&interfaces.interfaces, queryable, v);
			}
			(None, false) => {}
		}
	}
	async fn self_moved(&self) {
		for queryable in self
			.interesting_queryables
			.read()
			.await
			.keys()
			.flat_map(|v| v.upgrade())
		{
			// this also gets self.interested_interfaces, but it also gets it as
			// readonly, so its fine
			self.update_hit_queryable(&queryable).await;
		}
	}
	async fn init(self: &Arc<Self>) {
		_ = self.self_moved_handle.set(
			match &self.inner {
				QueryType::Zone {
					handler: _,
					field,
					margin: _,
					_shape_changed,
				} => {
					_ = _shape_changed.set(field.shape_changed_callback({
						let query = Arc::downgrade(self);
						move || {
							if let Some(q) = query.upgrade() {
								tokio::spawn(async move {
									q.self_moved().await;
								});
							}
						}
					}));
					&field.spatial
				}
				QueryType::Beam {
					handler: _,
					ref_space: origin,
					origin: _,
					dir: _,
					max_length: _,
				} => origin,
				QueryType::Points {
					handler: _,
					ref_space,
					points: _,
				} => ref_space,
			}
			.moved_callback({
				let query = Arc::downgrade(self);
				move || {
					if let Some(q) = query.upgrade() {
						tokio::spawn(async move {
							q.self_moved().await;
						});
					}
				}
			}),
		);
		let queryables = OwnedRegistry::new();
		for i in self.interfaces.iter() {
			let v = QUERY_STATE.interface_to_queryable.read().await;
			for q in v.get(&i.id).iter().flat_map(|v| v.get_valid_contents()) {
				queryables.add_raw(q);
			}
		}
		for q in queryables.get_vec() {
			self.update_interfaces(&q).await;
		}
		for (queryable, interfaces) in self
			.interesting_queryables
			.read()
			.await
			.iter()
			.flat_map(|(k, v)| Some((k.upgrade()?, v)))
		{
			if let Some(data) = self.inner.hit(&queryable).await {
				_ = self
					.inner
					.match_gained(&interfaces.interfaces, &queryable, data);
			}
		}
	}
}
impl Drop for Query {
	fn drop(&mut self) {
		QUERY_STATE.queries.remove(self);
	}
}
impl QueryType {
	/// this could take a while to run, might we worth to run in a spawn_blocking?
	async fn hit(&self, queryable: &Queryable) -> Option<HitTestResult> {
		match self {
			// TODO: improve this intersection test a bunch, this is probably completely wrong
			QueryType::Zone {
				handler: _,
				field,
				margin,
				_shape_changed: _,
			} => {
				if !queryable.spatial.visible() {
					return None;
				}
				if !field.spatial.visible() {
					return None;
				}
				let (_scale, _rotation, pos) =
					Spatial::space_to_space_matrix(Some(&queryable.spatial), Some(&field.spatial))
						.to_scale_rotation_translation();
				let distance = field.local_sample(pos.into()).distance;

				(distance < *margin).then_some(HitTestResult::Zone { pos, distance })
			}
			QueryType::Beam {
				handler: _,
				ref_space,
				origin,
				dir,
				max_length,
			} => {
				if !queryable.field.data.spatial.visible() {
					return None;
				}
				let ray_march = queryable.field.data.ray_march(Ray {
					origin: *origin,
					direction: *dir,
					space: ref_space.clone(),
				});
				(ray_march.min_distance <= 0.0 && ray_march.deepest_point_distance <= *max_length)
					.then_some(HitTestResult::Beam {
						deepest_point_distance: ray_march.deepest_point_distance,
						distance: ray_march.min_distance,
					})
			}
			QueryType::Points {
				handler: _,
				ref_space,
				points,
			} => {
				let distance = points
					.read()
					.await
					.iter()
					.map(|p| {
						let distance = queryable
							.field
							.data
							.sample(ref_space, p.point.into())
							.distance;
						(distance - p.margin, distance)
					})
					.reduce(|(sort1, distance1), (sort2, distance2)| {
						if sort1 < sort2 {
							(sort1, distance1)
						} else {
							(sort2, distance2)
						}
					});
				distance
					.filter(|(distance, _)| *distance < 0.0)
					.map(|(_, distance)| HitTestResult::Points { distance })
			}
		}
	}
	fn left(&self, obj: QueryableObjectRef) -> Result<(), gluon::SendError> {
		match self {
			QueryType::Zone {
				handler,
				field: _,
				margin: _,
				_shape_changed: _,
			} => handler.left(obj),
			QueryType::Beam {
				handler,
				ref_space: _,
				origin: _,
				dir: _,
				max_length: _,
			} => handler.left(obj),
			QueryType::Points {
				handler,
				ref_space: _,
				points: _,
			} => handler.left(obj),
		}
	}
	fn interfaces_changed(
		&self,
		obj: QueryableObjectRef,
		interfaces: Vec<QueriedInterface>,
	) -> Result<(), gluon::SendError> {
		match self {
			QueryType::Zone {
				handler,
				field: _,
				margin: _,
				_shape_changed: _,
			} => handler.interfaces_changed(obj, interfaces),
			QueryType::Beam {
				handler,
				ref_space: _,
				origin: _,
				dir: _,
				max_length: _,
			} => handler.interfaces_changed(obj, interfaces),
			QueryType::Points {
				handler,
				ref_space: _,
				points: _,
			} => handler.interfaces_changed(obj, interfaces),
		}
	}
	fn match_gained(
		&self,
		interfaces: &Registry<QueryableInterface>,
		queryable: &Arc<Queryable>,
		data: HitTestResult,
	) -> Result<(), gluon::SendError> {
		let interfaces = interfaces
			.get_valid_contents()
			.into_iter()
			.map(|v| QueriedInterface {
				interface_id: v.interface_id.get_string().clone(),
				interface: v.interface_ref.clone(),
			})
			.collect::<Vec<_>>();
		match (self, data) {
			(
				QueryType::Zone {
					handler,
					field: _,
					margin: _,
					_shape_changed: _,
				},
				HitTestResult::Zone { pos, distance },
			) => handler.entered(
				QueryableObjectRef::from_handler(&queryable.queryable_ref),
				FieldRefProxy::from_handler(&queryable.field),
				SpatialRefProxy::from_handler(&queryable.spatial),
				interfaces,
				pos.into(),
				distance,
			),
			(
				QueryType::Beam {
					handler,
					ref_space: _,
					origin: _,
					dir: _,
					max_length: _,
				},
				HitTestResult::Beam {
					deepest_point_distance,
					distance,
				},
			) => handler.intersected(
				QueryableObjectRef::from_handler(&queryable.queryable_ref),
				FieldRefProxy::from_handler(&queryable.field),
				SpatialRefProxy::from_handler(&queryable.spatial),
				interfaces,
				deepest_point_distance,
				distance,
			),
			(
				QueryType::Points {
					handler,
					ref_space: _,
					points: _,
				},
				HitTestResult::Points { distance },
			) => dbg!(handler.entered(
				QueryableObjectRef::from_handler(&queryable.queryable_ref),
				FieldRefProxy::from_handler(&queryable.field),
				SpatialRefProxy::from_handler(&queryable.spatial),
				interfaces,
				distance,
			)),
			_ => {
				error!("tried sending entered event with mismatching QueryType and HitTestResult");
				Ok(())
			}
		}
	}
	fn moved(
		&self,
		queryable: &Arc<Queryable>,
		data: HitTestResult,
	) -> Result<(), gluon::SendError> {
		match (self, data) {
			(
				QueryType::Zone {
					handler,
					field: _,
					margin: _,
					_shape_changed: _,
				},
				HitTestResult::Zone { pos, distance },
			) => handler.moved(
				QueryableObjectRef::from_handler(&queryable.queryable_ref),
				pos.into(),
				distance,
			),
			(
				QueryType::Beam {
					handler,
					ref_space: _,
					origin: _,
					dir: _,
					max_length: _,
				},
				HitTestResult::Beam {
					deepest_point_distance,
					distance,
				},
			) => handler.moved(
				QueryableObjectRef::from_handler(&queryable.queryable_ref),
				deepest_point_distance,
				distance,
			),
			(
				QueryType::Points {
					handler,
					ref_space: _,
					points: _,
				},
				HitTestResult::Points { distance },
			) => handler.moved(
				QueryableObjectRef::from_handler(&queryable.queryable_ref),
				distance,
			),
			_ => {
				error!("tried sending moved event with mismatching QueryType and HitTestResult");
				Ok(())
			}
		}
	}
}

#[derive(Debug)]
enum HitTestResult {
	Zone {
		pos: Vec3,
		distance: f32,
	},
	Beam {
		deepest_point_distance: f32,
		distance: f32,
	},
	Points {
		distance: f32,
	},
}

interface!(SpatialQueryInterface);
impl SpatialQueryInterfaceHandler for SpatialQueryInterface {
	async fn beam_query(&self, _ctx: gluon::Context, query: BeamQuery) -> SpatialQueryGuard {
		let BeamQuery {
			handler,
			interfaces,
			reference_spatial,
			direction,
			origin,
			max_length,
		} = query;
		let Some(ref_space) = reference_spatial.owned() else {
			// TODO: replace with returned error
			panic!("invalid SpatialRef used while creating a beam query");
		};
		let mut interface_ids = Vec::with_capacity(interfaces.len());
		let mut found_required = false;
		for i in interfaces {
			found_required |= !i.optional;
			interface_ids.push(InterfaceQuery {
				id: DedupedStr::get(i.id).await,
				optional: i.optional,
			});
		}
		if !found_required {
			// TODO: replace with returned error
			panic!("no required interface")
		}

		let query = QUERY_STATE.queries.add(Query {
			interfaces: interface_ids,
			inner: QueryType::Beam {
				handler,
				ref_space: (**ref_space).clone(),
				origin: origin.into(),
				dir: direction.into(),
				max_length,
			},
			interesting_queryables: RwLock::default(),
			matching_queryables: Registry::new(),
			self_moved_handle: OnceLock::new(),
		});
		query.init().await;
		let v = PION.register_object(Guard(query)).to_service();
		SpatialQueryGuard::from_handler(&v)
	}

	async fn zone_query(&self, _ctx: gluon::Context, query: ZoneQuery) -> SpatialQueryGuard {
		let ZoneQuery {
			handler,
			interfaces,
			zone_field,
			margin,
		} = query;
		let Some(field) = zone_field.owned() else {
			// TODO: replace with returned error
			panic!("invalid FieldRef used while creating a zone query");
		};
		let mut interface_ids = Vec::with_capacity(interfaces.len());
		let mut found_required = false;
		for i in interfaces {
			found_required |= !i.optional;
			interface_ids.push(InterfaceQuery {
				id: DedupedStr::get(i.id).await,
				optional: i.optional,
			});
		}
		if !found_required {
			// TODO: replace with returned error
			panic!("no required interface")
		}

		let query = QUERY_STATE.queries.add(Query {
			interfaces: interface_ids,
			inner: QueryType::Zone {
				handler,
				field: field.data.clone(),
				margin,
				_shape_changed: OnceLock::new(),
			},
			interesting_queryables: RwLock::default(),
			matching_queryables: Registry::new(),
			self_moved_handle: OnceLock::new(),
		});
		query.init().await;
		let v = PION.register_object(Guard(query)).to_service();
		SpatialQueryGuard::from_handler(&v)
	}

	async fn points_query(
		&self,
		_ctx: gluon::Context,
		query: PointsQuery,
	) -> stardust_xr_protocol::spatial_query::PointsQueryHandle {
		let PointsQuery {
			handler,
			interfaces,
			reference_spatial,
			points,
		} = query;
		let Some(ref_space) = reference_spatial.owned() else {
			// TODO: replace with returned error
			panic!("invalid SpatialRef used while creating a points query");
		};
		let mut interface_ids = Vec::with_capacity(interfaces.len());
		let mut found_required = false;
		for i in interfaces {
			found_required |= !i.optional;
			interface_ids.push(InterfaceQuery {
				id: DedupedStr::get(i.id).await,
				optional: i.optional,
			});
		}
		if !found_required {
			// TODO: replace with returned error
			panic!("no required interface")
		}
		let query = QUERY_STATE.queries.add(Query {
			interfaces: interface_ids,
			inner: QueryType::Points {
				handler,
				ref_space: (**ref_space).clone(),
				points: RwLock::new(points),
			},
			interesting_queryables: RwLock::default(),
			matching_queryables: Registry::new(),
			self_moved_handle: OnceLock::new(),
		});
		query.init().await;
		let v = PION.register_object(PointsQueryHandle(query)).to_service();
		stardust_xr_protocol::spatial_query::PointsQueryHandle::from_handler(&v)
	}
}
#[derive(Debug, Handler)]
struct PointsQueryHandle(Arc<Query>);
impl PointsQueryHandleHandler for PointsQueryHandle {
	async fn update_points(&self, _ctx: gluon::Context, points: Vec<Point>) {
		if let QueryType::Points {
			handler: _,
			ref_space: _,
			points: inner_points,
		} = &self.0.inner
		{
			*inner_points.write().await = points;
			self.0.self_moved().await;
		}
	}
}
#[expect(unused)]
#[derive(Debug, Handler)]
struct Guard(Arc<Query>);
impl SpatialQueryGuardHandler for Guard {}

#[derive(Debug, Deref)]
struct WeakPtrHash<T>(Weak<T>);
impl<T> Clone for WeakPtrHash<T> {
	fn clone(&self) -> Self {
		Self(self.0.clone())
	}
}
impl<T> Hash for WeakPtrHash<T> {
	fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
		state.write_usize(self.0.as_ptr().addr());
	}
}
impl<T> Eq for WeakPtrHash<T> {}
impl<T> PartialEq for WeakPtrHash<T> {
	fn eq(&self, other: &Self) -> bool {
		self.0.ptr_eq(&other.0)
	}
}

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

	// Mirrors QueryType::Zone hit() math (sans visibility checks).
	fn zone_check(queryable_spatial: &Spatial, zone_field: &Field, margin: f32) -> bool {
		let (_s, _r, pos) =
			Spatial::space_to_space_matrix(Some(queryable_spatial), Some(&zone_field.spatial))
				.to_scale_rotation_translation();
		let distance = zone_field.local_sample(pos.into()).distance;
		distance < margin
	}

	// Mirrors QueryType::Beam hit() math.
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

	// Mirrors QueryType::Points hit() math.
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
		let zone = make_field(Spatial::test_new(None, Mat4::IDENTITY), Shape::Sphere { radius: 1.0 });
		let queryable = make_spatial(0.0, 0.0, 0.5);
		assert!(zone_check(&queryable, &zone, 0.0));
	}

	#[test]
	fn zone_object_outside_sphere_misses() {
		let zone = make_field(Spatial::test_new(None, Mat4::IDENTITY), Shape::Sphere { radius: 1.0 });
		let queryable = make_spatial(2.0, 0.0, 0.0);
		assert!(!zone_check(&queryable, &zone, 0.0));
	}

	#[test]
	fn zone_margin_extends_detection_range() {
		let zone = make_field(Spatial::test_new(None, Mat4::IDENTITY), Shape::Sphere { radius: 1.0 });
		// 0.5 m outside the sphere surface — within margin 1.0
		let queryable = make_spatial(1.5, 0.0, 0.0);
		assert!(zone_check(&queryable, &zone, 1.0));
	}

	#[test]
	fn zone_object_beyond_margin_misses() {
		let zone = make_field(Spatial::test_new(None, Mat4::IDENTITY), Shape::Sphere { radius: 1.0 });
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
		let sphere = make_field(Spatial::test_new(None, Mat4::IDENTITY), Shape::Sphere { radius: 1.0 });
		// Beam from (-3,0,0) along +X — passes through sphere at origin
		assert!(beam_check(&sphere, &ref_space, Vec3::new(-3.0, 0.0, 0.0), Vec3::X, f32::MAX));
	}

	#[test]
	fn beam_misses_sphere_when_offset() {
		let ref_space = Spatial::test_new(None, Mat4::IDENTITY);
		let sphere = make_field(Spatial::test_new(None, Mat4::IDENTITY), Shape::Sphere { radius: 1.0 });
		// Beam offset 2 m on Y — clears the sphere entirely
		assert!(!beam_check(&sphere, &ref_space, Vec3::new(-3.0, 2.0, 0.0), Vec3::X, f32::MAX));
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
		let sphere = make_field(Spatial::test_new(None, Mat4::IDENTITY), Shape::Sphere { radius: 1.0 });
		let pts = vec![Point { point: Vec3::ZERO.into(), margin: 0.0 }];
		assert!(points_check(&sphere, &ref_space, &pts));
	}

	#[test]
	fn points_single_point_outside_sphere_misses() {
		let ref_space = Spatial::test_new(None, Mat4::IDENTITY);
		let sphere = make_field(Spatial::test_new(None, Mat4::IDENTITY), Shape::Sphere { radius: 1.0 });
		let pts = vec![Point { point: Vec3::new(3.0, 0.0, 0.0).into(), margin: 0.0 }];
		assert!(!points_check(&sphere, &ref_space, &pts));
	}

	#[test]
	fn points_margin_extends_detection_range() {
		let ref_space = Spatial::test_new(None, Mat4::IDENTITY);
		let sphere = make_field(Spatial::test_new(None, Mat4::IDENTITY), Shape::Sphere { radius: 1.0 });
		// Point 0.5 m outside sphere surface; margin = 1.0 → hit
		let pts = vec![Point { point: Vec3::new(1.5, 0.0, 0.0).into(), margin: 1.0 }];
		assert!(points_check(&sphere, &ref_space, &pts));
	}

	#[test]
	fn points_any_inside_causes_hit() {
		let ref_space = Spatial::test_new(None, Mat4::IDENTITY);
		let sphere = make_field(Spatial::test_new(None, Mat4::IDENTITY), Shape::Sphere { radius: 1.0 });
		let pts = vec![
			Point { point: Vec3::new(5.0, 0.0, 0.0).into(), margin: 0.0 },
			Point { point: Vec3::ZERO.into(), margin: 0.0 }, // inside
			Point { point: Vec3::new(-5.0, 0.0, 0.0).into(), margin: 0.0 },
		];
		assert!(points_check(&sphere, &ref_space, &pts));
	}

	#[test]
	fn points_all_outside_misses() {
		let ref_space = Spatial::test_new(None, Mat4::IDENTITY);
		let sphere = make_field(Spatial::test_new(None, Mat4::IDENTITY), Shape::Sphere { radius: 1.0 });
		let pts = vec![
			Point { point: Vec3::new(5.0, 0.0, 0.0).into(), margin: 0.0 },
			Point { point: Vec3::new(-5.0, 0.0, 0.0).into(), margin: 0.0 },
		];
		assert!(!points_check(&sphere, &ref_space, &pts));
	}

	// --- WeakPtrHash ---

	#[test]
	fn weak_ptr_hash_same_allocation_is_equal() {
		let arc: Arc<u32> = Arc::new(42);
		let h1 = WeakPtrHash(Arc::downgrade(&arc));
		let h2 = WeakPtrHash(Arc::downgrade(&arc));
		assert_eq!(h1, h2);
	}

	#[test]
	fn weak_ptr_hash_different_allocations_not_equal() {
		let a: Arc<u32> = Arc::new(1);
		let b: Arc<u32> = Arc::new(1);
		assert_ne!(WeakPtrHash(Arc::downgrade(&a)), WeakPtrHash(Arc::downgrade(&b)));
	}
}

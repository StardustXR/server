use std::{
	collections::HashMap,
	hash::{BuildHasher, Hash, Hasher, RandomState},
	sync::{Arc, OnceLock, Weak},
};

use bevy::prelude::Deref;
use glam::{Vec3, Vec3A};
use stardust_xr_protocol::{
	query::{QueriedInterface, QueryableObjectRef},
	spatial_query::{
		BeamQuery, BeamQueryHandler, SpatialQueryInterfaceHandler, ZoneQuery, ZoneQueryHandler,
	},
};
use stardust_xr_server_foundation::{
	deduped_string::DedupedStr,
	registry::{OwnedRegistry, Registry},
};
use tokio::sync::RwLock;
use tracing::{debug, error, warn};

use crate::{
	interface,
	nodes::{
		ProxyExt as _,
		fields::{Field, FieldTrait, R, Ray, ShapeChangedCallback},
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

struct QueryableInterest {
	interfaces: Registry<QueryableInterface>,
	_move_handle: MovedCallback,
	_shape_callback: ShapeChangedCallback,
}
pub(super) struct Query {
	interfaces: Vec<InterfaceQuery>,
	interesting_queryables: RwLock<HashMap<WeakPtrHash<Queryable>, QueryableInterest>>,
	matching_queryables: Registry<Queryable>,
	self_moved_handle: OnceLock<MovedCallback>,
	inner: QueryType,
}
enum QueryType {
	Zone {
		handler: ZoneQueryHandler,
		field: Arc<Field>,
		margin: f32,
		_shape_changed: OnceLock<ShapeChangedCallback>,
	},
	Beam {
		handler: BeamQueryHandler,
		origin: Arc<Spatial>,
		dir: Vec3,
		max_length: f32,
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
				if self.matching_queryables.contains(&queryable) {
					self.matching_queryables.remove(&queryable);
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
	async fn update_hit_queryable(&self, queryable: &Arc<Queryable>) {
		let interfaces_guard = self.interesting_queryables.read().await;
		let Some(interfaces) = interfaces_guard.get(&WeakPtrHash(Arc::downgrade(queryable))) else {
			warn!("tried to update hit state for queryable without interfaces of interest");
			return;
		};
		let r = self.inner.hit(&queryable);
		match (r, self.matching_queryables.contains(queryable)) {
			(None, true) => {
				self.matching_queryables.remove(queryable);
				_ = self
					.inner
					.left(QueryableObjectRef::from_handler(&queryable.queryable_ref));
			}
			(Some(v), true) => {
				_ = self.inner.moved(queryable, v);
			}
			(Some(v), false) => {
				self.matching_queryables.add_raw(queryable);
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
						let query = Arc::downgrade(&self);
						move || {
							if let Some(q) = query.upgrade() {
								tokio::spawn(async move {
									q.self_moved().await;
								});
							}
						}
					}));
					&***field.spatial
				}
				QueryType::Beam {
					handler: _,
					origin,
					dir: _,
					max_length: _,
				} => &origin,
			}
			.moved_callback({
				let query = Arc::downgrade(&self);
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
			if let Some(data) = self.inner.hit(&queryable) {
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
	fn hit(&self, queryable: &Queryable) -> Option<HitTestResult> {
		match self {
			// TODO: improve this intersection test a bunch, this is probably completely wrong
			QueryType::Zone {
				handler: _,
				field,
				margin,
				_shape_changed: _,
			} => {
				if !queryable.field.data.spatial.visible() {
					return None;
				}
				if !field.spatial.visible() {
					return None;
				}
				// let zone_point = field.closest_point(&queryable.field.spatial, Vec3A::ZERO, R);
				let target_point =
					queryable
						.field
						.data
						.closest_point(&field.spatial, Vec3A::ZERO, R);
				// let zone_point_distance = queryable.field.distance(&field.spatial, zone_point);
				let target_point_distance =
					field.distance(&queryable.field.data.spatial, target_point);
				debug!(target_point_distance, "checking zone intersection");
				let (_scale, _rotation, position) = Spatial::space_to_space_matrix(
					Some(&queryable.field.data.spatial),
					Some(&field.spatial),
				)
				.to_scale_rotation_translation();
				(target_point_distance < *margin).then(|| HitTestResult::Zone {
					pos: position,
					distance: target_point_distance,
				})
			}
			QueryType::Beam {
				handler: _,
				origin,
				dir,
				max_length,
			} => {
				if !queryable.field.data.spatial.visible() {
					return None;
				}
				let ray_march = queryable.field.data.ray_march(Ray {
					origin: Vec3::ZERO,
					direction: *dir,
					space: origin.clone(),
				});
				(ray_march.min_distance <= 0.0 && ray_march.deepest_point_distance <= *max_length)
					.then(|| HitTestResult::Beam {
						deepest_point_distance: ray_march.deepest_point_distance,
						distance: ray_march.min_distance,
					})
			}
		}
	}
	fn left(&self, obj: QueryableObjectRef) -> Result<(), gluon_wire::GluonSendError> {
		match self {
			QueryType::Zone {
				handler,
				field: _,
				margin: _,
				_shape_changed: _,
			} => handler.left(obj),
			QueryType::Beam {
				handler,
				origin: _,
				dir: _,
				max_length: _,
			} => handler.left(obj),
		}
	}
	fn interfaces_changed(
		&self,
		obj: QueryableObjectRef,
		interfaces: Vec<QueriedInterface>,
	) -> Result<(), gluon_wire::GluonSendError> {
		match self {
			QueryType::Zone {
				handler,
				field: _,
				margin: _,
				_shape_changed: _,
			} => handler.interfaces_changed(obj, interfaces),
			QueryType::Beam {
				handler,
				origin: _,
				dir: _,
				max_length: _,
			} => handler.interfaces_changed(obj, interfaces),
		}
	}
	fn match_gained(
		&self,
		interfaces: &Registry<QueryableInterface>,
		queryable: &Arc<Queryable>,
		data: HitTestResult,
	) -> Result<(), gluon_wire::GluonSendError> {
		let interfaces = interfaces
			.get_valid_contents()
			.into_iter()
			.map(|v| QueriedInterface {
				interface_id: v.interface_id.get_string().clone(),
				interface: v.interface_ref.clone(),
			})
			.collect();
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
				queryable.field_proxy.clone(),
				interfaces,
				pos.into(),
				distance,
			),
			(
				QueryType::Beam {
					handler,
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
				queryable.field_proxy.clone(),
				interfaces,
				deepest_point_distance,
				distance,
			),
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
	) -> Result<(), gluon_wire::GluonSendError> {
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
			_ => {
				error!("tried sending moved event with mismatching QueryType and HitTestResult");
				Ok(())
			}
		}
	}
}

enum HitTestResult {
	Zone {
		pos: Vec3,
		distance: f32,
	},
	Beam {
		deepest_point_distance: f32,
		distance: f32,
	},
}

interface!(SpatialQueryInterface);
impl SpatialQueryInterfaceHandler for SpatialQueryInterface {
	async fn beam_query(&self, _ctx: gluon_wire::GluonCtx, query: BeamQuery) {
		let BeamQuery {
			handler,
			interfaces,
			origin_spatial,
			direction,
			limit,
			max_length,
		} = query;
		let Some(origin) = origin_spatial.owned() else {
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
				origin: (**origin).clone(),
                dir: direction.mint(),
				max_length,
			},
			interesting_queryables: RwLock::default(),
			matching_queryables: Registry::new(),
			self_moved_handle: OnceLock::new(),
		});
		query.init().await;
		tokio::spawn(async move {
			let QueryType::Beam { handler, .. } = &query.inner else {
				unreachable!()
			};
			// TODO: wait for handler death to drop query
			let _ = handler;
			std::future::pending::<()>().await;
		});
	}

	async fn zone_query(&self, _ctx: gluon_wire::GluonCtx, query: ZoneQuery) {
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
		tokio::spawn(async move {
			let QueryType::Zone { handler, .. } = &query.inner else {
				unreachable!()
			};
			// TODO: wait for handler death to drop query
			let _ = handler;
			std::future::pending::<()>().await;
		});
	}
}

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

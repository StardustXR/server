use crate::{
	nodes::{fields::FieldObject, spatial::SpatialObject},
	query::{QueryInterface, spatial_query::SpatialQueryInterface},
};
use glam::{Mat4, Vec3};
use gluon::{Context, Handler, RefExt as _};
use stardust_xr_protocol::{
	field::{Field as FieldProxy, FieldRef as FieldRefProxy, FieldSample, RayMarchResult, Shape},
	query::{
		InterfaceDependency, QueriedInterface, QueryInterfaceHandler as _, QueryableId,
		QueryableInterface as QueryableInterfaceProxy, QueryableInterfaceHandler,
		QueryableInterfaceLocal, QueryableObject,
	},
	spatial::{PartialTransform, SpatialHandler as _, SpatialLocal, SpatialRef as SpatialRefProxy},
	spatial_query::{
		BeamQuery, BeamQueryHandler, BeamQueryHandlerHandler, Point, PointsQuery,
		PointsQueryHandler, PointsQueryHandlerHandler, SpatialQueryInterfaceHandler as _,
		ZoneQuery, ZoneQueryHandler, ZoneQueryHandlerHandler,
	},
	types::Vec3F,
};
use std::{
	path::PathBuf,
	sync::{Arc, LazyLock},
	time::Duration,
};
use tokio::sync::mpsc;

// Shared runtime so gluon's node loopers always have a valid runtime handle.
static RT: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
	tokio::runtime::Builder::new_multi_thread()
		.enable_all()
		.build()
		.unwrap()
});

fn ctx() -> Context {
	Context::new(None)
}

fn prefixes() -> Arc<Vec<PathBuf>> {
	Arc::new(vec![])
}

// --- test handler types ---

#[derive(Debug, Handler)]
struct DummyInterface;
impl QueryableInterfaceHandler for DummyInterface {}

#[derive(Debug, Clone)]
enum ZoneEvent {
	Entered { _sample: FieldSample },
	Left,
}

#[derive(Debug, Handler)]
struct TestZoneHandler(mpsc::Sender<ZoneEvent>);
impl ZoneQueryHandlerHandler for TestZoneHandler {
	async fn entered(
		&self,
		_ctx: Context,
		_obj: QueryableId,
		_field: FieldRefProxy,
		_spatial: SpatialRefProxy,
		_interfaces: Vec<QueriedInterface>,
		_pos: Vec3F,
		sample: FieldSample,
	) {
		let _ = self.0.send(ZoneEvent::Entered { _sample: sample }).await;
	}
	async fn interfaces_changed(
		&self,
		_ctx: Context,
		_obj: QueryableId,
		_interfaces: Vec<QueriedInterface>,
	) {
	}
	async fn moved(&self, _ctx: Context, _obj: QueryableId, _pos: Vec3F, _sample: FieldSample) {}
	async fn left(&self, _ctx: Context, _obj: QueryableId) {
		let _ = self.0.send(ZoneEvent::Left).await;
	}
}

#[derive(Debug, Clone)]
enum BeamEvent {
	Intersected { _march_result: RayMarchResult },
	Left,
}

#[derive(Debug, Handler)]
struct TestBeamHandler(mpsc::Sender<BeamEvent>);
impl BeamQueryHandlerHandler for TestBeamHandler {
	async fn intersected(
		&self,
		_ctx: Context,
		_obj: QueryableId,
		_field: FieldRefProxy,
		_spatial: SpatialRefProxy,
		_interfaces: Vec<QueriedInterface>,
		march_result: RayMarchResult,
	) {
		let _ = self
			.0
			.send(BeamEvent::Intersected {
				_march_result: march_result,
			})
			.await;
	}
	async fn interfaces_changed(
		&self,
		_ctx: Context,
		_obj: QueryableId,
		_interfaces: Vec<QueriedInterface>,
	) {
	}
	async fn moved(&self, _ctx: Context, _obj: QueryableId, _march_result: RayMarchResult) {}
	async fn left(&self, _ctx: Context, _obj: QueryableId) {
		let _ = self.0.send(BeamEvent::Left).await;
	}
}

#[derive(Debug, Clone)]
enum PointsEvent {
	Entered { _sample: FieldSample },
	Left,
}

#[derive(Debug, Handler)]
struct TestPointsHandler(mpsc::Sender<PointsEvent>);
impl PointsQueryHandlerHandler for TestPointsHandler {
	async fn entered(
		&self,
		_ctx: Context,
		_obj: QueryableId,
		_field: FieldRefProxy,
		_spatial: SpatialRefProxy,
		_interfaces: Vec<QueriedInterface>,
		sample: FieldSample,
	) {
		let _ = self.0.send(PointsEvent::Entered { _sample: sample }).await;
	}
	async fn interfaces_changed(
		&self,
		_ctx: Context,
		_obj: QueryableId,
		_interfaces: Vec<QueriedInterface>,
	) {
	}
	async fn moved(&self, _ctx: Context, _obj: QueryableId, _sample: FieldSample) {}
	async fn left(&self, _ctx: Context, _obj: QueryableId) {
		let _ = self.0.send(PointsEvent::Left).await;
	}
}

// --- helper ---

struct QueryableHandle {
	pub spatial: SpatialLocal<SpatialObject>,
	#[allow(dead_code)]
	field: gluon::LocalRef<FieldProxy, FieldObject>,
	#[allow(dead_code)]
	queryable: QueryableObject,
	#[allow(dead_code)]
	iface_obj: QueryableInterfaceLocal<DummyInterface>,
	pub interface_guard: QueryableInterfaceProxy,
}

async fn make_queryable(translation: Vec3, shape: Shape, iface_id: &str) -> QueryableHandle {
	let spatial = SpatialObject::new(None, Mat4::from_translation(translation));
	let field = FieldObject::new(spatial.handler().clone(), shape);

	let q_iface = QueryInterface::new(&prefixes());
	let queryable = q_iface
		.register_queryable(ctx(), spatial.proxy().clone(), field.proxy().clone())
		.await
		.expect("register_queryable failed");

	let iface_obj = QueryableInterfaceProxy::new_service(DummyInterface)
		.expect("failed to create dummy interface node");
	let interface_guard = queryable
		.add_interface(iface_obj.proxy(), iface_id)
		.await
		.expect("add_interface transaction failed")
		.expect("add_interface failed");

	QueryableHandle {
		spatial,
		field,
		queryable,
		iface_obj,
		interface_guard,
	}
}

const HIT: Duration = Duration::from_millis(500);
const NO_HIT: Duration = Duration::from_millis(200);

// === zone query ===

#[test]
fn zone_entered_when_queryable_inside() {
	RT.block_on(async {
		let (tx, mut rx) = mpsc::channel(4);
		let handler = ZoneQueryHandler::new_service(TestZoneHandler(tx))
			.expect("failed to create handler node");

		let zone_spatial = SpatialObject::new(None, Mat4::IDENTITY);
		let zone_field = FieldObject::new(
			zone_spatial.handler().clone(),
			Shape::Sphere { radius: 2.0 },
		);
		let zone_field_ref = zone_field.get_ref().clone();

		let sq = SpatialQueryInterface::new(&prefixes());
		let _guard = sq
			.zone_query(
				ctx(),
				ZoneQuery {
					handler: handler.proxy().clone(),
					interfaces: vec![InterfaceDependency {
						id: "e2e.zone.inside".into(),
						optional: false,
					}],
					zone_field: zone_field_ref,
					margin: 0.0,
				},
			)
			.await
			.expect("query registration failed");

		let _h = make_queryable(Vec3::ZERO, Shape::Sphere { radius: 0.5 }, "e2e.zone.inside").await;

		let ev = tokio::time::timeout(HIT, rx.recv())
			.await
			.expect("timed out waiting for entered")
			.expect("channel closed");
		assert!(matches!(ev, ZoneEvent::Entered { .. }));
	});
}

#[test]
fn zone_no_entered_when_queryable_outside() {
	RT.block_on(async {
		let (tx, mut rx) = mpsc::channel(4);
		let handler = ZoneQueryHandler::new_service(TestZoneHandler(tx))
			.expect("failed to create handler node");

		let zone_spatial = SpatialObject::new(None, Mat4::IDENTITY);
		let zone_field = FieldObject::new(
			zone_spatial.handler().clone(),
			Shape::Sphere { radius: 1.0 },
		);
		let zone_field_ref = zone_field.get_ref().clone();

		let sq = SpatialQueryInterface::new(&prefixes());
		let _guard = sq
			.zone_query(
				ctx(),
				ZoneQuery {
					handler: handler.proxy().clone(),
					interfaces: vec![InterfaceDependency {
						id: "e2e.zone.outside".into(),
						optional: false,
					}],
					zone_field: zone_field_ref,
					margin: 0.0,
				},
			)
			.await
			.expect("query registration failed");

		let _h = make_queryable(
			Vec3::new(5.0, 0.0, 0.0),
			Shape::Sphere { radius: 0.5 },
			"e2e.zone.outside",
		)
		.await;

		assert!(
			tokio::time::timeout(NO_HIT, rx.recv()).await.is_err(),
			"expected no entered for out-of-zone queryable"
		);
	});
}

#[test]
fn zone_left_fires_when_interface_removed() {
	RT.block_on(async {
		let (tx, mut rx) = mpsc::channel(4);
		let handler = ZoneQueryHandler::new_service(TestZoneHandler(tx))
			.expect("failed to create handler node");

		let zone_spatial = SpatialObject::new(None, Mat4::IDENTITY);
		let zone_field = FieldObject::new(
			zone_spatial.handler().clone(),
			Shape::Sphere { radius: 2.0 },
		);
		let zone_field_ref = zone_field.get_ref().clone();

		let sq = SpatialQueryInterface::new(&prefixes());
		let _guard = sq
			.zone_query(
				ctx(),
				ZoneQuery {
					handler: handler.proxy().clone(),
					interfaces: vec![InterfaceDependency {
						id: "e2e.zone.left".into(),
						optional: false,
					}],
					zone_field: zone_field_ref,
					margin: 0.0,
				},
			)
			.await
			.expect("query registration failed");

		let h = make_queryable(Vec3::ZERO, Shape::Sphere { radius: 0.5 }, "e2e.zone.left").await;

		tokio::time::timeout(HIT, rx.recv())
			.await
			.expect("timed out waiting for entered")
			.expect("channel closed");

		// Dropping the interface guard removes the required interface → left fires.
		drop(h.interface_guard);

		let ev = tokio::time::timeout(HIT, rx.recv())
			.await
			.expect("timed out waiting for left")
			.expect("channel closed");
		assert!(matches!(ev, ZoneEvent::Left));
	});
}

#[test]
fn zone_no_entered_wrong_interface() {
	RT.block_on(async {
		let (tx, mut rx) = mpsc::channel(4);
		let handler = ZoneQueryHandler::new_service(TestZoneHandler(tx))
			.expect("failed to create handler node");

		let zone_spatial = SpatialObject::new(None, Mat4::IDENTITY);
		let zone_field = FieldObject::new(
			zone_spatial.handler().clone(),
			Shape::Sphere { radius: 2.0 },
		);
		let zone_field_ref = zone_field.get_ref().clone();

		let sq = SpatialQueryInterface::new(&prefixes());
		let _guard = sq
			.zone_query(
				ctx(),
				ZoneQuery {
					handler: handler.proxy().clone(),
					interfaces: vec![InterfaceDependency {
						id: "e2e.zone.required".into(),
						optional: false,
					}],
					zone_field: zone_field_ref,
					margin: 0.0,
				},
			)
			.await
			.expect("query registration failed");

		// Queryable has wrong interface ID
		let _h = make_queryable(Vec3::ZERO, Shape::Sphere { radius: 0.5 }, "e2e.zone.wrong").await;

		assert!(
			tokio::time::timeout(NO_HIT, rx.recv()).await.is_err(),
			"expected no entered for wrong interface"
		);
	});
}

#[test]
fn zone_left_when_queryable_moves_out() {
	RT.block_on(async {
		let (tx, mut rx) = mpsc::channel(4);
		let handler = ZoneQueryHandler::new_service(TestZoneHandler(tx))
			.expect("failed to create handler node");

		let zone_spatial = SpatialObject::new(None, Mat4::IDENTITY);
		let zone_field = FieldObject::new(
			zone_spatial.handler().clone(),
			Shape::Sphere { radius: 2.0 },
		);
		let zone_field_ref = zone_field.get_ref().clone();

		let sq = SpatialQueryInterface::new(&prefixes());
		let _guard = sq
			.zone_query(
				ctx(),
				ZoneQuery {
					handler: handler.proxy().clone(),
					interfaces: vec![InterfaceDependency {
						id: "e2e.zone.move_out".into(),
						optional: false,
					}],
					zone_field: zone_field_ref,
					margin: 0.0,
				},
			)
			.await
			.expect("query registration failed");

		let h = make_queryable(
			Vec3::ZERO,
			Shape::Sphere { radius: 0.5 },
			"e2e.zone.move_out",
		)
		.await;

		tokio::time::timeout(HIT, rx.recv())
			.await
			.expect("timed out waiting for entered")
			.expect("channel closed");

		// Move the queryable well outside the zone → left fires.
		h.spatial
			.set_local_transform(
				ctx(),
				PartialTransform {
					translation: Some(Vec3F {
						x: 50.0,
						y: 0.0,
						z: 0.0,
					}),
					rotation: None,
					scale: None,
				},
			)
			.await;

		let ev = tokio::time::timeout(HIT, rx.recv())
			.await
			.expect("timed out waiting for left")
			.expect("channel closed");
		assert!(matches!(ev, ZoneEvent::Left));
	});
}

#[test]
fn zone_left_when_queryable_hidden() {
	RT.block_on(async {
		let (tx, mut rx) = mpsc::channel(4);
		let handler = ZoneQueryHandler::new_service(TestZoneHandler(tx))
			.expect("failed to create handler node");

		let zone_spatial = SpatialObject::new(None, Mat4::IDENTITY);
		let zone_field = FieldObject::new(
			zone_spatial.handler().clone(),
			Shape::Sphere { radius: 2.0 },
		);
		let zone_field_ref = zone_field.get_ref().clone();

		let sq = SpatialQueryInterface::new(&prefixes());
		let _guard = sq
			.zone_query(
				ctx(),
				ZoneQuery {
					handler: handler.proxy().clone(),
					interfaces: vec![InterfaceDependency {
						id: "e2e.zone.hidden".into(),
						optional: false,
					}],
					zone_field: zone_field_ref,
					margin: 0.0,
				},
			)
			.await
			.expect("query registration failed");

		let h = make_queryable(Vec3::ZERO, Shape::Sphere { radius: 0.5 }, "e2e.zone.hidden").await;

		tokio::time::timeout(HIT, rx.recv())
			.await
			.expect("timed out waiting for entered")
			.expect("channel closed");

		// Zero scale hides the queryable → left fires even though it never moved.
		h.spatial
			.set_local_transform(
				ctx(),
				PartialTransform {
					translation: None,
					rotation: None,
					scale: Some(Vec3F {
						x: 0.0,
						y: 0.0,
						z: 0.0,
					}),
				},
			)
			.await;

		let ev = tokio::time::timeout(HIT, rx.recv())
			.await
			.expect("timed out waiting for left")
			.expect("channel closed");
		assert!(matches!(ev, ZoneEvent::Left));
	});
}

#[test]
fn zone_left_when_queryable_reparented_away() {
	RT.block_on(async {
		let (tx, mut rx) = mpsc::channel(4);
		let handler = ZoneQueryHandler::new_service(TestZoneHandler(tx))
			.expect("failed to create handler node");

		let zone_spatial = SpatialObject::new(None, Mat4::IDENTITY);
		let zone_field = FieldObject::new(
			zone_spatial.handler().clone(),
			Shape::Sphere { radius: 2.0 },
		);
		let zone_field_ref = zone_field.get_ref().clone();

		let sq = SpatialQueryInterface::new(&prefixes());
		let _guard = sq
			.zone_query(
				ctx(),
				ZoneQuery {
					handler: handler.proxy().clone(),
					interfaces: vec![InterfaceDependency {
						id: "e2e.zone.reparent".into(),
						optional: false,
					}],
					zone_field: zone_field_ref,
					margin: 0.0,
				},
			)
			.await
			.expect("query registration failed");

		let h = make_queryable(
			Vec3::ZERO,
			Shape::Sphere { radius: 0.5 },
			"e2e.zone.reparent",
		)
		.await;

		tokio::time::timeout(HIT, rx.recv())
			.await
			.expect("timed out waiting for entered")
			.expect("channel closed");

		// Reparent under a far-away parent — the queryable's local transform is
		// unchanged, but its global pose leaves the zone → left fires.
		let far_parent =
			SpatialObject::new(None, Mat4::from_translation(Vec3::new(100.0, 0.0, 0.0)));
		h.spatial
			.set_parent(ctx(), far_parent.get_ref().proxy().clone())
			.await;

		let ev = tokio::time::timeout(HIT, rx.recv())
			.await
			.expect("timed out waiting for left")
			.expect("channel closed");
		assert!(matches!(ev, ZoneEvent::Left));
	});
}

// End-to-end latency of a zone transition: queryable moves → moved callback →
// spawned reconcile → handler event delivered. Run with:
// cargo test --profile benching -- --ignored bench_ --nocapture --test-threads=1
#[test]
#[ignore]
fn bench_query_e2e_zone_transition_roundtrip() {
	RT.block_on(async {
		let (tx, mut rx) = mpsc::channel(4);
		let handler = ZoneQueryHandler::new_service(TestZoneHandler(tx)).expect("failed to create handler node");

		let zone_spatial = SpatialObject::new(None, Mat4::IDENTITY);
		let zone_field = FieldObject::new(zone_spatial.handler().clone(), Shape::Sphere { radius: 2.0 });
		let zone_field_ref = zone_field.get_ref().clone();

		let sq = SpatialQueryInterface::new(&prefixes());
		let _guard = sq.zone_query(ctx(), ZoneQuery {
			handler: handler.proxy().clone(),
			interfaces: vec![InterfaceDependency { id: "bench.zone.roundtrip".into(), optional: false }],
			zone_field: zone_field_ref,
			margin: 0.0,
		}).await;

		let h = make_queryable(Vec3::ZERO, Shape::Sphere { radius: 0.5 }, "bench.zone.roundtrip").await;
		tokio::time::timeout(HIT, rx.recv()).await
			.expect("timed out waiting for initial entered")
			.expect("channel closed");

		let translate = |x: f32| PartialTransform {
			translation: Some(Vec3F { x, y: 0.0, z: 0.0 }),
			rotation: None,
			scale: None,
		};

		const TRANSITIONS: u32 = 400;
		let start = std::time::Instant::now();
		for _ in 0..TRANSITIONS / 2 {
			h.spatial.set_local_transform(ctx(), translate(50.0)).await;
			let ev = tokio::time::timeout(HIT, rx.recv()).await
				.expect("timed out waiting for left")
				.expect("channel closed");
			assert!(matches!(ev, ZoneEvent::Left));

			h.spatial.set_local_transform(ctx(), translate(0.0)).await;
			let ev = tokio::time::timeout(HIT, rx.recv()).await
				.expect("timed out waiting for entered")
				.expect("channel closed");
			assert!(matches!(ev, ZoneEvent::Entered { .. }));
		}
		println!(
			"zone e2e transition roundtrip                        {:>10.1} µs/transition   ({TRANSITIONS} transitions)",
			start.elapsed().as_micros() as f64 / f64::from(TRANSITIONS)
		);
	});
}

// === beam query ===

#[test]
fn beam_intersected_when_queryable_in_path() {
	RT.block_on(async {
		let (tx, mut rx) = mpsc::channel(4);
		let handler = BeamQueryHandler::new_service(TestBeamHandler(tx))
			.expect("failed to create handler node");

		let ref_spatial = SpatialObject::new(None, Mat4::IDENTITY);

		let sq = SpatialQueryInterface::new(&prefixes());
		let _guard = sq
			.beam_query(
				ctx(),
				BeamQuery {
					handler: handler.proxy().clone(),
					interfaces: vec![InterfaceDependency {
						id: "e2e.beam.hit".into(),
						optional: false,
					}],
					reference_spatial: ref_spatial.get_ref().proxy().clone(),
					origin: Vec3F {
						x: -5.0,
						y: 0.0,
						z: 0.0,
					},
					direction: Vec3F {
						x: 1.0,
						y: 0.0,
						z: 0.0,
					},
					max_length: f32::MAX,
				},
			)
			.await
			.expect("query registration failed");

		// Sphere at origin; beam along +X from −5 passes through it.
		let _h = make_queryable(Vec3::ZERO, Shape::Sphere { radius: 1.0 }, "e2e.beam.hit").await;

		let ev = tokio::time::timeout(HIT, rx.recv())
			.await
			.expect("timed out waiting for intersected")
			.expect("channel closed");
		assert!(matches!(ev, BeamEvent::Intersected { .. }));
	});
}

#[test]
fn beam_no_intersected_when_queryable_offset() {
	RT.block_on(async {
		let (tx, mut rx) = mpsc::channel(4);
		let handler = BeamQueryHandler::new_service(TestBeamHandler(tx))
			.expect("failed to create handler node");

		let ref_spatial = SpatialObject::new(None, Mat4::IDENTITY);

		let sq = SpatialQueryInterface::new(&prefixes());
		let _guard = sq
			.beam_query(
				ctx(),
				BeamQuery {
					handler: handler.proxy().clone(),
					interfaces: vec![InterfaceDependency {
						id: "e2e.beam.miss".into(),
						optional: false,
					}],
					reference_spatial: ref_spatial.get_ref().proxy().clone(),
					origin: Vec3F {
						x: -5.0,
						y: 0.0,
						z: 0.0,
					},
					direction: Vec3F {
						x: 1.0,
						y: 0.0,
						z: 0.0,
					},
					max_length: f32::MAX,
				},
			)
			.await
			.expect("query registration failed");

		// Sphere offset 5 m on Y — beam misses entirely.
		let _h = make_queryable(
			Vec3::new(0.0, 5.0, 0.0),
			Shape::Sphere { radius: 1.0 },
			"e2e.beam.miss",
		)
		.await;

		assert!(
			tokio::time::timeout(NO_HIT, rx.recv()).await.is_err(),
			"expected no intersected for offset queryable"
		);
	});
}

// === points query ===

#[test]
fn points_entered_when_point_inside_field() {
	RT.block_on(async {
		let (tx, mut rx) = mpsc::channel(4);
		let handler = PointsQueryHandler::new_service(TestPointsHandler(tx))
			.expect("failed to create handler node");

		let ref_spatial = SpatialObject::new(None, Mat4::IDENTITY);

		let sq = SpatialQueryInterface::new(&prefixes());
		let _handle = sq
			.points_query(
				ctx(),
				PointsQuery {
					handler: handler.proxy().clone(),
					interfaces: vec![InterfaceDependency {
						id: "e2e.points.hit".into(),
						optional: false,
					}],
					reference_spatial: ref_spatial.get_ref().proxy().clone(),
					points: vec![Point {
						point: Vec3F {
							x: 0.0,
							y: 0.0,
							z: 0.0,
						},
						margin: 0.0,
					}],
				},
			)
			.await
			.expect("query registration failed");

		// Sphere at origin, point (0,0,0) is inside.
		let _h = make_queryable(Vec3::ZERO, Shape::Sphere { radius: 1.0 }, "e2e.points.hit").await;

		let ev = tokio::time::timeout(HIT, rx.recv())
			.await
			.expect("timed out waiting for entered")
			.expect("channel closed");
		assert!(matches!(ev, PointsEvent::Entered { .. }));
	});
}

#[test]
fn points_no_entered_when_queryable_hidden() {
	RT.block_on(async {
		let (tx, mut rx) = mpsc::channel(4);
		let handler = PointsQueryHandler::new_service(TestPointsHandler(tx))
			.expect("failed to create handler node");

		let ref_spatial = SpatialObject::new(None, Mat4::IDENTITY);

		// Queryable would match the point, but is hidden by zero scale first.
		let h = make_queryable(
			Vec3::ZERO,
			Shape::Sphere { radius: 1.0 },
			"e2e.points.hidden",
		)
		.await;
		h.spatial
			.set_local_transform(
				ctx(),
				PartialTransform {
					translation: None,
					rotation: None,
					scale: Some(Vec3F {
						x: 0.0,
						y: 0.0,
						z: 0.0,
					}),
				},
			)
			.await;

		let sq = SpatialQueryInterface::new(&prefixes());
		let _handle = sq
			.points_query(
				ctx(),
				PointsQuery {
					handler: handler.proxy().clone(),
					interfaces: vec![InterfaceDependency {
						id: "e2e.points.hidden".into(),
						optional: false,
					}],
					reference_spatial: ref_spatial.get_ref().proxy().clone(),
					points: vec![Point {
						point: Vec3F {
							x: 0.0,
							y: 0.0,
							z: 0.0,
						},
						margin: 0.0,
					}],
				},
			)
			.await
			.expect("query registration failed");

		assert!(
			tokio::time::timeout(NO_HIT, rx.recv()).await.is_err(),
			"expected no entered for hidden queryable"
		);
	});
}

#[test]
fn points_no_entered_when_point_outside_field() {
	RT.block_on(async {
		let (tx, mut rx) = mpsc::channel(4);
		let handler = PointsQueryHandler::new_service(TestPointsHandler(tx))
			.expect("failed to create handler node");

		let ref_spatial = SpatialObject::new(None, Mat4::IDENTITY);

		let sq = SpatialQueryInterface::new(&prefixes());
		let _handle = sq
			.points_query(
				ctx(),
				PointsQuery {
					handler: handler.proxy().clone(),
					interfaces: vec![InterfaceDependency {
						id: "e2e.points.miss".into(),
						optional: false,
					}],
					reference_spatial: ref_spatial.get_ref().proxy().clone(),
					points: vec![Point {
						point: Vec3F {
							x: 5.0,
							y: 0.0,
							z: 0.0,
						},
						margin: 0.0,
					}],
				},
			)
			.await
			.expect("query registration failed");

		// Sphere at origin; point (5,0,0) is outside.
		let _h = make_queryable(Vec3::ZERO, Shape::Sphere { radius: 1.0 }, "e2e.points.miss").await;

		assert!(
			tokio::time::timeout(NO_HIT, rx.recv()).await.is_err(),
			"expected no entered for outside point"
		);
	});
}

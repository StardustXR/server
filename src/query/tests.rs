use crate::{
	PION,
	nodes::{fields::FieldObject, spatial::SpatialObject},
	query::{QueryInterface, spatial_query::SpatialQueryInterface},
};
use glam::{Mat4, Vec3};
use gluon::{Context, Handler, Object};
use std::{path::PathBuf, sync::{Arc, LazyLock}, time::Duration};
use stardust_xr_protocol::{
	field::{Field as FieldProxy, FieldHandler, Shape},
	query::{
		InterfaceDependency, QueryInterfaceHandler, QueryableInterfaceGuard,
		QueryableInterfaceGuardHandler, QueriedInterface, QueryableObjectRef,
	},
	spatial::Spatial as SpatialProxy,
	spatial_query::{
		BeamQuery, BeamQueryHandler, BeamQueryHandlerHandler,
		PointsQuery, PointsQueryHandler, PointsQueryHandlerHandler,
		SpatialQueryInterfaceHandler, ZoneQuery, ZoneQueryHandler, ZoneQueryHandlerHandler,
		Point,
	},
	types::Vec3F,
	field::FieldRef as FieldRefProxy,
	spatial::SpatialRef as SpatialRefProxy,
};
use tokio::sync::mpsc;

// Shared runtime so PION's binder looper threads always have a valid runtime handle.
static RT: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
	tokio::runtime::Builder::new_multi_thread()
		.enable_all()
		.build()
		.unwrap()
});

fn ctx() -> Context {
	Context { sender_pid: 0, sender_euid: 0 }
}

fn prefixes() -> Arc<Vec<PathBuf>> {
	Arc::new(vec![])
}

// --- test handler types ---

#[derive(Debug, Handler)]
struct DummyInterface;
impl QueryableInterfaceGuardHandler for DummyInterface {}

#[derive(Debug, Clone)]
enum ZoneEvent {
	Entered { distance: f32 },
	Left,
}

#[derive(Debug, Handler)]
struct TestZoneHandler(mpsc::Sender<ZoneEvent>);
impl ZoneQueryHandlerHandler for TestZoneHandler {
	async fn entered(
		&self, _ctx: Context,
		_obj: QueryableObjectRef, _field: FieldRefProxy, _spatial: SpatialRefProxy,
		_interfaces: Vec<QueriedInterface>, _pos: Vec3F, distance: f32,
	) {
		let _ = self.0.send(ZoneEvent::Entered { distance }).await;
	}
	async fn interfaces_changed(&self, _ctx: Context, _obj: QueryableObjectRef, _interfaces: Vec<QueriedInterface>) {}
	async fn moved(&self, _ctx: Context, _obj: QueryableObjectRef, _pos: Vec3F, _distance: f32) {}
	async fn left(&self, _ctx: Context, _obj: QueryableObjectRef) {
		let _ = self.0.send(ZoneEvent::Left).await;
	}
}

#[derive(Debug, Clone)]
enum BeamEvent {
	Intersected { distance: f32 },
	Left,
}

#[derive(Debug, Handler)]
struct TestBeamHandler(mpsc::Sender<BeamEvent>);
impl BeamQueryHandlerHandler for TestBeamHandler {
	async fn intersected(
		&self, _ctx: Context,
		_obj: QueryableObjectRef, _field: FieldRefProxy, _spatial: SpatialRefProxy,
		_interfaces: Vec<QueriedInterface>, _deepest: f32, distance: f32,
	) {
		let _ = self.0.send(BeamEvent::Intersected { distance }).await;
	}
	async fn interfaces_changed(&self, _ctx: Context, _obj: QueryableObjectRef, _interfaces: Vec<QueriedInterface>) {}
	async fn moved(&self, _ctx: Context, _obj: QueryableObjectRef, _deepest: f32, _distance: f32) {}
	async fn left(&self, _ctx: Context, _obj: QueryableObjectRef) {
		let _ = self.0.send(BeamEvent::Left).await;
	}
}

#[derive(Debug, Clone)]
enum PointsEvent {
	Entered { distance: f32 },
	Left,
}

#[derive(Debug, Handler)]
struct TestPointsHandler(mpsc::Sender<PointsEvent>);
impl PointsQueryHandlerHandler for TestPointsHandler {
	async fn entered(
		&self, _ctx: Context,
		_obj: QueryableObjectRef, _field: FieldRefProxy, _spatial: SpatialRefProxy,
		_interfaces: Vec<QueriedInterface>, distance: f32,
	) {
		let _ = self.0.send(PointsEvent::Entered { distance }).await;
	}
	async fn interfaces_changed(&self, _ctx: Context, _obj: QueryableObjectRef, _interfaces: Vec<QueriedInterface>) {}
	async fn moved(&self, _ctx: Context, _obj: QueryableObjectRef, _distance: f32) {}
	async fn left(&self, _ctx: Context, _obj: QueryableObjectRef) {
		let _ = self.0.send(PointsEvent::Left).await;
	}
}

// --- helper ---

struct QueryableHandle {
	#[allow(dead_code)]
	spatial: gluon::ObjectRef<SpatialObject>,
	#[allow(dead_code)]
	field: gluon::ObjectRef<FieldObject>,
	#[allow(dead_code)]
	queryable: stardust_xr_protocol::query::QueryableObject,
	#[allow(dead_code)]
	iface_obj: Object<DummyInterface>,
	pub interface_guard: QueryableInterfaceGuard,
}

async fn make_queryable(translation: Vec3, shape: Shape, iface_id: &str) -> QueryableHandle {
	let spatial = SpatialObject::new(None, Mat4::from_translation(translation));
	let field = FieldObject::new(spatial.clone(), shape);

	let q_iface = QueryInterface::new(&prefixes());
	let queryable = q_iface
		.register_queryable(ctx(), SpatialProxy::from_handler(&spatial), FieldProxy::from_handler(&field))
		.await
		.expect("register_queryable failed");

	let iface_obj = PION.register_object(DummyInterface);
	let interface_guard = queryable.add_interface(&iface_obj, iface_id).await.expect("add_interface failed");

	QueryableHandle { spatial, field, queryable, iface_obj, interface_guard }
}

const HIT: Duration = Duration::from_millis(500);
const NO_HIT: Duration = Duration::from_millis(200);

// === zone query ===

#[test]
fn zone_entered_when_queryable_inside() {
	RT.block_on(async {
		let (tx, mut rx) = mpsc::channel(4);
		let handler = PION.register_object(TestZoneHandler(tx)).to_service();

		let zone_spatial = SpatialObject::new(None, Mat4::IDENTITY);
		let zone_field = FieldObject::new(zone_spatial.clone(), Shape::Sphere { radius: 2.0 });
		let zone_field_ref = zone_field.field_ref(ctx()).await;

		let sq = SpatialQueryInterface::new(&prefixes());
		let _guard = sq.zone_query(ctx(), ZoneQuery {
			handler: ZoneQueryHandler::from_handler(&handler),
			interfaces: vec![InterfaceDependency { id: "e2e.zone.inside".into(), optional: false }],
			zone_field: zone_field_ref,
			margin: 0.0,
		}).await;

		let _h = make_queryable(Vec3::ZERO, Shape::Sphere { radius: 0.5 }, "e2e.zone.inside").await;

		let ev = tokio::time::timeout(HIT, rx.recv()).await
			.expect("timed out waiting for entered")
			.expect("channel closed");
		assert!(matches!(ev, ZoneEvent::Entered { .. }));
	});
}

#[test]
fn zone_no_entered_when_queryable_outside() {
	RT.block_on(async {
		let (tx, mut rx) = mpsc::channel(4);
		let handler = PION.register_object(TestZoneHandler(tx)).to_service();

		let zone_spatial = SpatialObject::new(None, Mat4::IDENTITY);
		let zone_field = FieldObject::new(zone_spatial.clone(), Shape::Sphere { radius: 1.0 });
		let zone_field_ref = zone_field.field_ref(ctx()).await;

		let sq = SpatialQueryInterface::new(&prefixes());
		let _guard = sq.zone_query(ctx(), ZoneQuery {
			handler: ZoneQueryHandler::from_handler(&handler),
			interfaces: vec![InterfaceDependency { id: "e2e.zone.outside".into(), optional: false }],
			zone_field: zone_field_ref,
			margin: 0.0,
		}).await;

		let _h = make_queryable(Vec3::new(5.0, 0.0, 0.0), Shape::Sphere { radius: 0.5 }, "e2e.zone.outside").await;

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
		let handler = PION.register_object(TestZoneHandler(tx)).to_service();

		let zone_spatial = SpatialObject::new(None, Mat4::IDENTITY);
		let zone_field = FieldObject::new(zone_spatial.clone(), Shape::Sphere { radius: 2.0 });
		let zone_field_ref = zone_field.field_ref(ctx()).await;

		let sq = SpatialQueryInterface::new(&prefixes());
		let _guard = sq.zone_query(ctx(), ZoneQuery {
			handler: ZoneQueryHandler::from_handler(&handler),
			interfaces: vec![InterfaceDependency { id: "e2e.zone.left".into(), optional: false }],
			zone_field: zone_field_ref,
			margin: 0.0,
		}).await;

		let h = make_queryable(Vec3::ZERO, Shape::Sphere { radius: 0.5 }, "e2e.zone.left").await;

		tokio::time::timeout(HIT, rx.recv()).await
			.expect("timed out waiting for entered")
			.expect("channel closed");

		// Dropping the interface guard removes the required interface → left fires.
		drop(h.interface_guard);

		let ev = tokio::time::timeout(HIT, rx.recv()).await
			.expect("timed out waiting for left")
			.expect("channel closed");
		assert!(matches!(ev, ZoneEvent::Left));
	});
}

#[test]
fn zone_no_entered_wrong_interface() {
	RT.block_on(async {
		let (tx, mut rx) = mpsc::channel(4);
		let handler = PION.register_object(TestZoneHandler(tx)).to_service();

		let zone_spatial = SpatialObject::new(None, Mat4::IDENTITY);
		let zone_field = FieldObject::new(zone_spatial.clone(), Shape::Sphere { radius: 2.0 });
		let zone_field_ref = zone_field.field_ref(ctx()).await;

		let sq = SpatialQueryInterface::new(&prefixes());
		let _guard = sq.zone_query(ctx(), ZoneQuery {
			handler: ZoneQueryHandler::from_handler(&handler),
			interfaces: vec![InterfaceDependency { id: "e2e.zone.required".into(), optional: false }],
			zone_field: zone_field_ref,
			margin: 0.0,
		}).await;

		// Queryable has wrong interface ID
		let _h = make_queryable(Vec3::ZERO, Shape::Sphere { radius: 0.5 }, "e2e.zone.wrong").await;

		assert!(
			tokio::time::timeout(NO_HIT, rx.recv()).await.is_err(),
			"expected no entered for wrong interface"
		);
	});
}

// === beam query ===

#[test]
fn beam_intersected_when_queryable_in_path() {
	RT.block_on(async {
		let (tx, mut rx) = mpsc::channel(4);
		let handler = PION.register_object(TestBeamHandler(tx)).to_service();

		let ref_spatial = SpatialObject::new(None, Mat4::IDENTITY);

		let sq = SpatialQueryInterface::new(&prefixes());
		let _guard = sq.beam_query(ctx(), BeamQuery {
			handler: BeamQueryHandler::from_handler(&handler),
			interfaces: vec![InterfaceDependency { id: "e2e.beam.hit".into(), optional: false }],
			reference_spatial: SpatialRefProxy::from_handler(ref_spatial.get_ref()),
			origin: Vec3F { x: -5.0, y: 0.0, z: 0.0 },
			direction: Vec3F { x: 1.0, y: 0.0, z: 0.0 },
			max_length: f32::MAX,
		}).await;

		// Sphere at origin; beam along +X from −5 passes through it.
		let _h = make_queryable(Vec3::ZERO, Shape::Sphere { radius: 1.0 }, "e2e.beam.hit").await;

		let ev = tokio::time::timeout(HIT, rx.recv()).await
			.expect("timed out waiting for intersected")
			.expect("channel closed");
		assert!(matches!(ev, BeamEvent::Intersected { .. }));
	});
}

#[test]
fn beam_no_intersected_when_queryable_offset() {
	RT.block_on(async {
		let (tx, mut rx) = mpsc::channel(4);
		let handler = PION.register_object(TestBeamHandler(tx)).to_service();

		let ref_spatial = SpatialObject::new(None, Mat4::IDENTITY);

		let sq = SpatialQueryInterface::new(&prefixes());
		let _guard = sq.beam_query(ctx(), BeamQuery {
			handler: BeamQueryHandler::from_handler(&handler),
			interfaces: vec![InterfaceDependency { id: "e2e.beam.miss".into(), optional: false }],
			reference_spatial: SpatialRefProxy::from_handler(ref_spatial.get_ref()),
			origin: Vec3F { x: -5.0, y: 0.0, z: 0.0 },
			direction: Vec3F { x: 1.0, y: 0.0, z: 0.0 },
			max_length: f32::MAX,
		}).await;

		// Sphere offset 5 m on Y — beam misses entirely.
		let _h = make_queryable(Vec3::new(0.0, 5.0, 0.0), Shape::Sphere { radius: 1.0 }, "e2e.beam.miss").await;

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
		let handler = PION.register_object(TestPointsHandler(tx)).to_service();

		let ref_spatial = SpatialObject::new(None, Mat4::IDENTITY);

		let sq = SpatialQueryInterface::new(&prefixes());
		let _handle = sq.points_query(ctx(), PointsQuery {
			handler: PointsQueryHandler::from_handler(&handler),
			interfaces: vec![InterfaceDependency { id: "e2e.points.hit".into(), optional: false }],
			reference_spatial: SpatialRefProxy::from_handler(ref_spatial.get_ref()),
			points: vec![Point { point: Vec3F { x: 0.0, y: 0.0, z: 0.0 }, margin: 0.0 }],
		}).await;

		// Sphere at origin, point (0,0,0) is inside.
		let _h = make_queryable(Vec3::ZERO, Shape::Sphere { radius: 1.0 }, "e2e.points.hit").await;

		let ev = tokio::time::timeout(HIT, rx.recv()).await
			.expect("timed out waiting for entered")
			.expect("channel closed");
		assert!(matches!(ev, PointsEvent::Entered { .. }));
	});
}

#[test]
fn points_no_entered_when_point_outside_field() {
	RT.block_on(async {
		let (tx, mut rx) = mpsc::channel(4);
		let handler = PION.register_object(TestPointsHandler(tx)).to_service();

		let ref_spatial = SpatialObject::new(None, Mat4::IDENTITY);

		let sq = SpatialQueryInterface::new(&prefixes());
		let _handle = sq.points_query(ctx(), PointsQuery {
			handler: PointsQueryHandler::from_handler(&handler),
			interfaces: vec![InterfaceDependency { id: "e2e.points.miss".into(), optional: false }],
			reference_spatial: SpatialRefProxy::from_handler(ref_spatial.get_ref()),
			points: vec![Point { point: Vec3F { x: 5.0, y: 0.0, z: 0.0 }, margin: 0.0 }],
		}).await;

		// Sphere at origin; point (5,0,0) is outside.
		let _h = make_queryable(Vec3::ZERO, Shape::Sphere { radius: 1.0 }, "e2e.points.miss").await;

		assert!(
			tokio::time::timeout(NO_HIT, rx.recv()).await.is_err(),
			"expected no entered for outside point"
		);
	});
}

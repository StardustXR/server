#![allow(unused)]

use std::{
	any::{Any, type_name},
	collections::HashSet,
	fmt::Debug,
	fs::{File, OpenOptions},
	marker::PhantomData,
	path::Path,
	sync::{
		Arc,
		atomic::{AtomicBool, Ordering},
	},
};

use bevy::prelude::{Deref, DerefMut};
use gluon::{Handler, IntoHandler, RefExt, RefFsBinding};
use stardust_xr_protocol::{
	spatial::SpatialRef,
	tracked::{TrackedGuardHandler, TrackedHandler, TrackedStateReceiver},
	types::{Posef, Timestamp},
};
use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use tracing::{error, info};

use crate::{
	STARDUST_INSTANCE,
	nodes::{ProxyExt, spatial::Spatial},
};

// FIX ORDER: 3
// pub mod hmd;
// FIX ORDER: 4
// pub mod input;
// FIX ORDER: 3
// pub mod stage;
// pub mod play_space;

#[derive(Debug)]
pub struct Tracked<T: Debug + Send + Sync + 'static> {
	inner: gluon::Node<TrackedInner<T>>,
	binding: RefFsBinding,
	_type: PhantomData<T>,
}
#[derive(Debug, Handler)]
struct TrackedInner<T: Debug + Send + Sync + 'static> {
	tracked: AtomicBool,
	receivers: Arc<RwLock<HashSet<TrackedStateReceiver>>>,
	spatial: SpatialRef,
	data: RwLock<T>,
	pose_callback: fn(&T, &Spatial, Timestamp) -> (Option<Posef>, bool),
}

impl<T: Debug + Send + Sync + 'static> Tracked<T> {
	pub fn new(
		spatial: SpatialRef,
		pose_getter: fn(&T, &Spatial, Timestamp) -> (Option<Posef>, bool),
		tracked: bool,
		pion_dir: &str,
		data: T,
	) -> Option<Self> {
		let path = stardust_xr_protocol::dir::server_file_path(pion_dir, STARDUST_INSTANCE.wait())?;
		let (inner, tracked) = stardust_xr_protocol::tracked::Tracked::new_node(TrackedInner {
			tracked: AtomicBool::new(tracked),
			receivers: Arc::default(),
			spatial,
			pose_callback: pose_getter,
			data: RwLock::new(data),
		})
		.inspect_err(|err| error!("failed to create Tracked node: {err}"))
		.ok()?;
		info!("creating Tracked at {path:?}");
		let binding = tracked
			.proxy()
			.bind(&path)
			.inspect_err(|err| error!(?path, "failed to bind tracked to path: {err}"))
			.ok()?;

		Some(Self {
			inner,
			_type: PhantomData,
			binding,
		})
	}
	pub fn tracked_blocking(&self, tracked: bool) {
		self.inner.tracked.store(tracked, Ordering::Relaxed);
		// TODO: move this to a task or something to not block a core thread?
		let receivers = self.inner.receivers.blocking_read();
		for recv in receivers.iter() {
			_ = recv.tracked(tracked);
		}
	}
	pub fn get_data_blocking(&self) -> RwLockReadGuard<'_, T> {
		self.inner.data.blocking_read()
	}
	pub fn get_mut_data_blocking(&self) -> RwLockWriteGuard<'_, T> {
		self.inner.data.blocking_write()
	}
}
impl<T: Debug + Send + Sync + 'static> TrackedHandler for TrackedInner<T> {
	async fn get(
		&self,
		_ctx: gluon::Context,
		handler: TrackedStateReceiver,
	) -> (
		SpatialRef,
		stardust_xr_protocol::tracked::TrackedGuard,
		bool,
	) {
		self.receivers.write().await.insert(handler.clone());
		(
			self.spatial.clone(),
			stardust_xr_protocol::tracked::TrackedGuard::new_service(
				(TrackedGuard(handler, self.receivers.clone())),
			)
			// TODO: don't unwrap, maybe don't have this at all?
			.unwrap()
			.into_proxy(),
			self.tracked.load(Ordering::Relaxed),
		)
	}

	async fn get_pose(
		&self,
		_ctx: gluon::Context,
		at: Timestamp,
		relative_to: SpatialRef,
	) -> (Option<Posef>, bool) {
		let Some(spatial) = relative_to.owned() else {
			return (None, false);
		};
		let state = self.data.read().await;
		(self.pose_callback)(&state, &spatial, at)
	}
}
#[derive(Debug, Handler)]
struct TrackedGuard(
	TrackedStateReceiver,
	Arc<RwLock<HashSet<TrackedStateReceiver>>>,
);
impl TrackedGuardHandler for TrackedGuard {}

#[derive(Deref, DerefMut)]
struct DebugWrapper<T>(T);
impl<T> From<T> for DebugWrapper<T> {
	fn from(value: T) -> Self {
		Self(value)
	}
}
impl<T> Debug for DebugWrapper<T> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_tuple("DebugWrapper")
			.field(&type_name::<T>())
			.finish()
	}
}

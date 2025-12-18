#![allow(unused)]

use super::{Message, MessageSink, display::Display};
use crate::wayland::{Client, WaylandError, WaylandResult, core::surface::Surface};
use parking_lot::Mutex;
use stardust_xr_server_foundation::registry::Registry;
use tracing::info;
use std::{
	fmt::Debug,
	sync::{Arc, Weak},
};
use waynest::ObjectId;
use waynest_protocols::server::core::wayland::wl_display::WlDisplay;
use waynest_server::{Client as _, RequestDispatcher};

pub trait ClientExt {
	fn message_sink(&self) -> MessageSink;
	fn display(&self) -> Arc<Display>;
	fn try_get<D: RequestDispatcher>(&self, id: ObjectId) -> WaylandResult<Arc<D>>;
}
impl ClientExt for Client {
	fn message_sink(&self) -> MessageSink {
		self.get::<Display>(ObjectId::DISPLAY)
			.unwrap()
			.message_sink
			.clone()
	}

	fn display(&self) -> Arc<Display> {
		self.get::<Display>(ObjectId::DISPLAY).unwrap()
	}

	fn try_get<D: RequestDispatcher>(&self, id: ObjectId) -> WaylandResult<Arc<D>> {
		self.get::<D>(id).ok_or(WaylandError::MissingObject(id))
	}
}

// #[derive(Debug, Default)]
// pub struct DoubleBuffer<State: Debug + Clone> {
// 	pub current: State,
// 	pub pending: State,
// }
// impl<State: Debug + Clone> DoubleBuffer<State> {
// 	pub fn new(initial_state: State) -> Self {
// 		DoubleBuffer {
// 			current: initial_state.clone(),
// 			pending: initial_state,
// 		}
// 	}
// 	pub fn apply(&mut self) {
// 		self.current = self.pending.clone();
// 	}
// 	pub fn current(&self) -> &State {
// 		&self.current
// 	}
// }

#[derive(Debug)]
pub struct SurfaceCommitAwareBufferManager {
	registry: Mutex<Vec<Box<dyn SurfaceCommitAwareBufferFns>>>,
	surface: Weak<Surface>,
}
impl SurfaceCommitAwareBufferManager {
	pub fn new(surface: Weak<Surface>) -> Arc<Self> {
		Arc::new(Self {
			registry: Default::default(),
			surface,
		})
	}
	pub fn update_current(&self) {
		info!("pre lock");
		let mut lock = self.registry.lock();
		info!("post lock");
		lock.retain(|v| v.valid());
		lock.iter().for_each(|v| v.update_current());
	}
	pub fn requires_surface_syncronization(&self) -> bool {
		if let Some(surface) = self.surface.upgrade() {
			false
		} else {
			false
		}
	}
}
trait SurfaceCommitAwareBufferFns: Send + Sync + 'static + Debug {
	fn valid(&self) -> bool;
	fn update_current(&self);
}

impl<State: BufferedState> SurfaceCommitAwareBufferFns
	for Weak<Mutex<SurfaceCommitAwareBuffer<State>>>
{
	fn valid(&self) -> bool {
		self.strong_count() > 0
	}

	fn update_current(&self) {
		if let Some(v) = self.upgrade() {
			v.lock().update_current();
		};
	}
}

#[derive(Debug)]
pub struct SurfaceCommitAwareBuffer<State: BufferedState> {
	pub current: State,
	pub applied: State,
	pub pending: State,
	buffer_manager: Arc<SurfaceCommitAwareBufferManager>,
}
impl<State: BufferedState> SurfaceCommitAwareBuffer<State> {
	pub fn new(initial_state: State, surface: &Surface) -> Arc<Mutex<Self>> {
		Self::new_from_manager(initial_state, surface.get_state_buffer_manager())
	}
	pub fn new_from_manager(
		initial_state: State,
		buffer_manager: Arc<SurfaceCommitAwareBufferManager>,
	) -> Arc<Mutex<Self>> {
		let v = Arc::new(
			Self {
				pending: initial_state.get_initial_pending(),
				applied: initial_state.get_initial_pending(),
				current: initial_state,
				buffer_manager: buffer_manager.clone(),
			}
			.into(),
		);
		buffer_manager
			.registry
			.lock()
			.push(Box::new(Arc::downgrade(&v)));

		v
	}
	pub fn apply(&mut self) {
		self.applied.apply(&mut self.pending);
		if !self.buffer_manager.requires_surface_syncronization() {
			self.update_current();
		}
	}
	pub fn update_current(&mut self) {
		self.current.apply(&mut self.applied);
	}
	pub fn current(&self) -> &State {
		&self.current
	}
}

pub trait BufferedState: Debug + Send + Sync + 'static {
	/// applies the pending changes to self
	fn apply(&mut self, pending: &mut Self);
	fn get_initial_pending(&self) -> Self;
}

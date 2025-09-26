#![allow(unused)]

use super::{Message, MessageSink, display::Display};
use crate::wayland::{Client, WaylandError, WaylandResult};
use std::{fmt::Debug, sync::Arc};
use waynest::ObjectId;
use waynest_protocols::server::core::wayland::wl_display::WlDisplay;
use waynest_server::RequestDispatcher;

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

#[derive(Debug, Default)]
pub struct DoubleBuffer<State: Debug + Clone> {
	pub current: State,
	pub pending: State,
}
impl<State: Debug + Clone> DoubleBuffer<State> {
	pub fn new(initial_state: State) -> Self {
		DoubleBuffer {
			current: initial_state.clone(),
			pending: initial_state,
		}
	}
	pub fn apply(&mut self) {
		self.current = self.pending.clone();
	}
	pub fn current(&self) -> &State {
		&self.current
	}
}

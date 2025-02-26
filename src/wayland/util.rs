#![allow(unused)]

use super::{MessageSink, core::display::Display};
use std::fmt::Debug;
use waynest::{server::Client, wire::ObjectId};

pub trait ClientExt {
	fn message_sink(&self) -> MessageSink;
}
impl ClientExt for Client {
	fn message_sink(&self) -> MessageSink {
		self.get::<Display>(ObjectId::DISPLAY)
			.unwrap()
			.message_sink
			.clone()
	}
}

// pub trait ObjectIdExt {
// 	fn upgrade(&self, client: &Client) -> Option<Object>;
// }
// impl ObjectIdExt for ObjectId {
// 	fn upgrade(&self, client: &Client) -> Option<Object> {
// 		client.get_object(self)
// 	}
// }

#[derive(Debug, Default)]
pub struct DoubleBuffer<State: Debug + Clone> {
	current: State,
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

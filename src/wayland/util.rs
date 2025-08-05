#![allow(unused)]

use super::{Message, MessageSink, display::Display};
use std::{fmt::Debug, sync::Arc};
use waynest::{
	server::{Client, Result, protocol::core::wayland::wl_display::WlDisplay},
	wire::ObjectId,
};

pub trait ClientExt {
	fn message_sink(&self) -> MessageSink;
	fn display(&self) -> Arc<Display>;
	async fn protocol_error(
		&mut self,
		sender_id: ObjectId,
		object_id: ObjectId,
		code: u32,
		message: String,
	) -> Result<()>;
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

	async fn protocol_error(
		&mut self,
		sender_id: ObjectId,
		object_id: ObjectId,
		code: u32,
		message: String,
	) -> Result<()> {
		self.display()
			.error(self, sender_id, object_id, code, message)
			.await?;
		let _ = self.message_sink().send(Message::Disconnect);

		Ok(())
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

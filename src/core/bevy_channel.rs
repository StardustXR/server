use std::sync::OnceLock;

use bevy::prelude::*;
use tokio::sync::mpsc::{self, error::TryRecvError};

#[derive(Resource)]
pub struct BevyChannelReader<T: Send + Sync + 'static>(mpsc::UnboundedReceiver<T>);
pub struct BevyChannel<T: Send + Sync + 'static>(OnceLock<mpsc::UnboundedSender<T>>);
impl<T: Send + Sync + 'static> BevyChannel<T> {
	pub const fn new() -> Self {
		Self(OnceLock::new())
	}
	pub fn init(&self, app: &mut App) {
		let (tx, rx) = mpsc::unbounded_channel();
		self.0.set(tx).unwrap();
		app.insert_resource(BevyChannelReader(rx));
	}
	pub fn send(&self, msg: T) -> Option<()> {
		self.0.get()?.send(msg).ok()
	}
}

impl<T: Send + Sync + 'static> BevyChannelReader<T> {
	pub fn read(&mut self) -> Option<T> {
		match self.0.try_recv() {
			Ok(v) => Some(v),
			Err(TryRecvError::Disconnected) => panic!("bevy channel should never disconnect"),
			Err(TryRecvError::Empty) => None,
		}
	}
}


use super::core::Node;
use crate::core::eventloop::EventLoop;
use anyhow::{ensure, Result};
use rccell::RcCell;
use std::sync::{Arc, RwLock, Weak};

pub struct PulseSender {
	event_loop: Weak<EventLoop>,
	registry_idx: RwLock<Option<usize>>,
}

impl PulseSender {
	pub fn add_to(node: &RcCell<Node>) -> Result<()> {
		ensure!(
			node.borrow().spatial.is_some(),
			"Node does not have a spatial attached!"
		);

		let sender = Arc::new(PulseSender {
			event_loop: node.borrow().get_client().map_or(Weak::new(), |client| {
				Arc::downgrade(&client.get_event_loop())
			}),
			registry_idx: RwLock::new(None),
		});
		let idx = sender
			.event_loop
			.upgrade()
			.and_then(|event_loop| event_loop.pulse_senders.add(sender.clone()).ok());
		*sender.registry_idx.write().unwrap() = idx;
		Ok(())
	}
}

impl Drop for PulseSender {
	fn drop(&mut self) {
		let event_loop = self.event_loop.upgrade();
		let idx = self
			.registry_idx
			.write()
			.ok()
			.and_then(|registry_idx| registry_idx.clone());
		event_loop
			.zip(idx)
			.and_then(|(event_loop, idx)| event_loop.pulse_senders.remove(idx).ok());
	}
}

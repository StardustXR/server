use super::core::Node;
use crate::core::registry::{Registry, RegistryEntry};
use anyhow::{ensure, Result};
use lazy_static::lazy_static;
use rccell::RcCell;
use std::sync::RwLock;

lazy_static! {
	static ref PULSE_SENDER_REGISTRY: Registry<PulseSender> = Default::default();
}

pub struct PulseSender {
	registry_idx: RwLock<usize>,
}

impl PulseSender {
	pub fn add_to(node: &RcCell<Node>) -> Result<()> {
		ensure!(
			node.borrow().spatial.is_some(),
			"Node does not have a spatial attached!"
		);

		let sender = PulseSender {
			registry_idx: Default::default(),
		};
		let sender = PULSE_SENDER_REGISTRY.add(sender)?;
		node.borrow_mut().pulse_sender = Some(sender);
		Ok(())
	}
}

impl RegistryEntry for PulseSender {
	fn store_idx(&self, store_idx: usize) {
		*self.registry_idx.write().unwrap() = store_idx;
	}
}

impl Drop for PulseSender {
	fn drop(&mut self) {
		let _ = PULSE_SENDER_REGISTRY.remove(*self.registry_idx.read().unwrap());
	}
}

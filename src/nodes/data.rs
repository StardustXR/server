use super::core::Node;
use crate::core::registry::Registry;
use anyhow::{ensure, Result};
use lazy_static::lazy_static;
use std::sync::Arc;

lazy_static! {
	static ref PULSE_SENDER_REGISTRY: Registry<PulseSender> = Default::default();
}

pub struct PulseSender {}

impl PulseSender {
	pub fn add_to(node: &Arc<Node>) -> Result<()> {
		ensure!(
			node.spatial.read().is_some(),
			"Internal: Node does not have a spatial attached!"
		);

		let sender = PulseSender {};
		let sender = PULSE_SENDER_REGISTRY.add(sender)?;
		*node.pulse_sender.write() = Some(sender);
		Ok(())
	}
}

impl Drop for PulseSender {
	fn drop(&mut self) {
		let _ = PULSE_SENDER_REGISTRY.remove(self);
	}
}

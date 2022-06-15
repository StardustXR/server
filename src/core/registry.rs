use anyhow::{anyhow, Result};
use parking_lot::RwLock;
use slab::Slab;
use std::sync::{Arc, Weak};

pub struct Registry<T>(RwLock<Slab<Weak<T>>>);

impl<T> Registry<T> {
	pub fn add(&self, t: T) -> Result<Arc<T>> {
		let t_arc = Arc::new(t);
		self.0.write().insert(Arc::downgrade(&t_arc));
		Ok(t_arc)
	}
	pub fn get_valid_contents(&self) -> Vec<Arc<T>> {
		self.0
			.read()
			.iter()
			.filter_map(|(_, item)| item.upgrade())
			.collect()
	}
	pub fn remove(&self, t: &T) -> Result<()> {
		let mut del_idx: Option<usize> = None;
		for item in self.0.read().iter() {
			let (idx, item) = item;
			if let Some(item) = item.upgrade() {
				if std::ptr::eq(item.as_ref(), t) {
					del_idx = Some(idx);
					break;
				}
			}
		}
		del_idx
			.map(|idx| self.0.write().remove(idx))
			.ok_or_else(|| anyhow!("Node not found to remove"))?;
		Ok(())
	}
}

impl<T> Default for Registry<T> {
	fn default() -> Self {
		Registry::<T>(RwLock::new(Slab::new()))
	}
}

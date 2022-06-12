use anyhow::{anyhow, Result};
use slab::{Iter, Slab};
use std::sync::Arc;
use std::sync::RwLock;

pub trait RegistryEntry {
	fn store_idx(&self, idx: usize);
}

pub struct Registry<T: RegistryEntry>(RwLock<Slab<Arc<T>>>);

impl<T: RegistryEntry> Registry<T> {
	pub fn add(&self, t: T) -> Result<Arc<T>> {
		let t_arc = Arc::new(t);
		let idx = self
			.0
			.write()
			.ok()
			.ok_or_else(|| anyhow!("Registry has been poisoned"))?
			.insert(t_arc.clone());
		t_arc.store_idx(idx);
		Ok(t_arc)
	}
	pub fn iterate<F: FnOnce(Iter<'_, Arc<T>>)>(&self, closure: F) -> Result<()> {
		closure(
			self.0
				.read()
				.ok()
				.ok_or_else(|| anyhow!("Registry has been poisoned"))?
				.iter(),
		);
		Ok(())
	}
	pub fn remove(&self, index: usize) -> Result<T, Arc<T>> {
		Arc::try_unwrap(self.0.write().unwrap().remove(index))
	}
}

impl<T: RegistryEntry> Default for Registry<T> {
	fn default() -> Self {
		Registry::<T>(RwLock::new(Slab::new()))
	}
}

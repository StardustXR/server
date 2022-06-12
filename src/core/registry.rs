use anyhow::{anyhow, Result};
use slab::{Iter, Slab};
use std::sync::RwLock;

struct Registry<T>(RwLock<Slab<T>>);

impl<T> Registry<T> {
	pub fn add(&self, t: T) -> Result<usize> {
		Ok(self
			.0
			.write()
			.ok()
			.ok_or_else(|| anyhow!("Registry has been poisoned"))?
			.insert(t))
	}
	pub fn iterate<F: FnOnce(Iter<'_, T>)>(&self, index: usize, closure: F) -> Result<()> {
		closure(
			self.0
				.read()
				.ok()
				.ok_or_else(|| anyhow!("Registry has been poisoned"))?
				.iter(),
		);
		Ok(())
	}
	pub fn remove(&self, index: usize) -> Result<T> {
		Ok(self
			.0
			.write()
			.ok()
			.ok_or_else(|| anyhow!("Registry has been poisoned"))?
			.remove(index))
	}
}

impl<T> Default for Registry<T> {
	fn default() -> Self {
		Registry::<T>(RwLock::new(Slab::new()))
	}
}

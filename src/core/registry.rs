use anyhow::{anyhow, Result};
use parking_lot::RwLock;
use slab::Slab;
use std::sync::{Arc, Weak};

pub struct Registry<T>(RwLock<Slab<Weak<T>>>);

impl<T> Registry<T> {
	pub fn add(&self, t: T) -> Arc<T> {
		let t_arc = Arc::new(t);
		self.add_raw(&t_arc);
		t_arc
	}
	pub fn add_raw(&self, t: &Arc<T>) {
		self.0.write().insert(Arc::downgrade(t));
	}
	pub fn get_valid_contents(&self) -> Vec<Arc<T>> {
		self.0
			.read()
			.iter()
			.filter_map(|(_, item)| item.upgrade())
			.collect()
	}
	pub fn remove(&self, t: &T) {
		let mut del_idx: Option<usize> = None;
		for item in self.0.read().iter() {
			let (idx, item) = item;
			if let Some(item) = item.upgrade() {
				if std::ptr::eq(item.as_ref(), t) {
					self.0.write().remove(idx);
					break;
				}
			}
		}
	}
}

impl<T> Default for Registry<T> {
	fn default() -> Self {
		Registry::<T>(RwLock::new(Slab::new()))
	}
}

use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use slab::Slab;
use std::sync::{Arc, Weak};

pub struct Registry<T>(Mutex<Slab<Weak<T>>>);

impl<T> Registry<T> {
	pub fn add(&self, t: T) -> Arc<T> {
		let t_arc = Arc::new(t);
		self.add_raw(&t_arc);
		t_arc
	}
	pub fn add_raw(&self, t: &Arc<T>) {
		self.0.lock().insert(Arc::downgrade(t));
	}
	pub fn get_valid_contents(&self) -> Vec<Arc<T>> {
		self.0
			.lock()
			.iter()
			.filter_map(|(_, item)| item.upgrade())
			.collect()
	}
	pub fn remove(&self, t: &T) {
		for item in self.0.lock().iter() {
			let (idx, item) = item;
			if let Some(item) = item.upgrade() {
				if std::ptr::eq(item.as_ref(), t) {
					self.0.lock().remove(idx);
					break;
				}
			}
		}
	}
}

impl<T> Default for Registry<T> {
	fn default() -> Self {
		Registry::<T>(Mutex::new(Slab::new()))
	}
}

#![allow(dead_code)]

use std::ptr;
use std::sync::{Arc, Weak};

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;

pub struct Registry<T>(Lazy<Mutex<FxHashMap<usize, Weak<T>>>>);

impl<T: Send + Sync> Registry<T> {
	pub const fn new() -> Self {
		Registry(Lazy::new(|| Mutex::new(FxHashMap::default())))
	}
	pub fn add(&self, t: T) -> Arc<T> {
		let t_arc = Arc::new(t);
		self.add_raw(&t_arc);
		t_arc
	}
	pub fn add_raw(&self, t: &Arc<T>) {
		self.0
			.lock()
			.insert(Arc::as_ptr(t) as usize, Arc::downgrade(t));
	}
	pub fn get_valid_contents(&self) -> Vec<Arc<T>> {
		self.0
			.lock()
			.iter()
			.filter_map(|pair| pair.1.upgrade())
			.collect()
	}
	pub fn remove(&self, t: &T) {
		self.0.lock().remove(&(ptr::addr_of!(*t) as usize));
	}
	pub fn clear(&self) {
		self.0.lock().clear();
	}
}

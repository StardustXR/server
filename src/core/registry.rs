#![allow(dead_code)]

use std::ptr;
use std::sync::{Arc, Weak};

use core::hash::BuildHasherDefault;
use dashmap::DashMap;
use rustc_hash::FxHasher;

pub struct Registry<T>(DashMap<usize, Weak<T>, BuildHasherDefault<FxHasher>>);

impl<T> Registry<T> {
	pub fn add(&self, t: T) -> Arc<T> {
		let t_arc = Arc::new(t);
		self.add_raw(&t_arc);
		t_arc
	}
	pub fn add_raw(&self, t: &Arc<T>) {
		self.0
			.insert(ptr::addr_of!(**t) as usize, Arc::downgrade(t));
	}
	pub fn get_valid_contents(&self) -> Vec<Arc<T>> {
		self.0
			.iter()
			.filter_map(|pair| pair.value().upgrade())
			.collect()
	}
	pub fn remove(&self, t: &T) {
		self.0.remove(&(ptr::addr_of!(*t) as usize));
	}
	pub fn clear(&self) {
		self.0.clear();
	}
}

impl<T> Default for Registry<T> {
	fn default() -> Self {
		Registry(DashMap::default())
	}
}

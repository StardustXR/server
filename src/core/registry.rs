#![allow(dead_code)]

use parking_lot::{const_mutex, MappedMutexGuard, Mutex, MutexGuard};
use rustc_hash::FxHashMap;
use std::ptr;
use std::sync::{Arc, Weak};

pub struct Registry<T: Send + Sync + ?Sized>(Mutex<Option<FxHashMap<usize, Weak<T>>>>);

impl<T: Send + Sync + ?Sized> Registry<T> {
	pub const fn new() -> Self {
		Registry(const_mutex(None))
	}
	fn lock(&self) -> MappedMutexGuard<FxHashMap<usize, Weak<T>>> {
		MutexGuard::map(self.0.lock(), |r| {
			r.get_or_insert_with(|| FxHashMap::default())
		})
	}
	pub fn add(&self, t: T) -> Arc<T>
	where
		T: Sized,
	{
		let t_arc = Arc::new(t);
		self.add_raw(&t_arc);
		t_arc
	}
	pub fn add_raw(&self, t: &Arc<T>) {
		self.lock()
			.insert(Arc::as_ptr(t) as *const () as usize, Arc::downgrade(t));
	}
	pub fn contains(&self, t: &T) -> bool {
		self.lock()
			.contains_key(&(ptr::addr_of!(*t) as *const () as usize))
	}
	pub fn get_valid_contents(&self) -> Vec<Arc<T>> {
		self.lock()
			.iter()
			.filter_map(|pair| pair.1.upgrade())
			.collect()
	}
	pub fn take_valid_contents(&self) -> Vec<Arc<T>> {
		self.0
			.lock()
			.take()
			.unwrap_or_default()
			.into_iter()
			.filter_map(|pair| pair.1.upgrade())
			.collect()
	}
	pub fn remove(&self, t: &T) {
		self.lock()
			.remove(&(ptr::addr_of!(*t) as *const () as usize));
	}
	pub fn clear(&self) {
		self.lock().clear();
	}
}
impl<T: Send + Sync + ?Sized> Clone for Registry<T> {
	fn clone(&self) -> Self {
		Self(Mutex::new(self.0.lock().clone()))
	}
}

pub struct OwnedRegistry<T: Send + Sync + ?Sized>(Mutex<Option<FxHashMap<usize, Arc<T>>>>);

impl<T: Send + Sync + ?Sized> OwnedRegistry<T> {
	pub const fn new() -> Self {
		OwnedRegistry(const_mutex(None))
	}
	fn lock(&self) -> MappedMutexGuard<FxHashMap<usize, Arc<T>>> {
		MutexGuard::map(self.0.lock(), |r| {
			r.get_or_insert_with(|| FxHashMap::default())
		})
	}
	pub fn add(&self, t: T) -> Arc<T>
	where
		T: Sized,
	{
		let t_arc = Arc::new(t);
		self.add_raw(t_arc.clone());
		t_arc
	}
	pub fn add_raw(&self, t: Arc<T>) {
		self.lock().insert(Arc::as_ptr(&t) as *const () as usize, t);
	}
	pub fn get_vec(&self) -> Vec<Arc<T>> {
		self.lock().values().cloned().collect::<Vec<_>>()
	}
	pub fn contains(&self, t: &T) -> bool {
		self.lock()
			.contains_key(&(ptr::addr_of!(*t) as *const () as usize))
	}
	pub fn remove(&self, t: &T) {
		self.lock()
			.remove(&(ptr::addr_of!(*t) as *const () as usize));
	}
	pub fn clear(&self) {
		self.lock().clear();
	}
}
impl<T: Send + Sync + ?Sized> Clone for OwnedRegistry<T> {
	fn clone(&self) -> Self {
		Self(Mutex::new(self.0.lock().clone()))
	}
}

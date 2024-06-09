#![allow(dead_code)]

use parking_lot::{const_mutex, MappedMutexGuard, Mutex, MutexGuard};
use rustc_hash::FxHashMap;
use std::ptr;
use std::sync::{Arc, Weak};

#[derive(Debug)]
pub struct Registry<T: Send + Sync + ?Sized>(Mutex<Option<FxHashMap<usize, Weak<T>>>>);

impl<T: Send + Sync + ?Sized> Registry<T> {
	pub const fn new() -> Self {
		Registry(const_mutex(None))
	}
	fn lock(&self) -> MappedMutexGuard<FxHashMap<usize, Weak<T>>> {
		MutexGuard::map(self.0.lock(), |r| {
			r.get_or_insert_with(FxHashMap::default)
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
	pub fn get_changes(old: &Registry<T>, new: &Registry<T>) -> (Vec<Arc<T>>, Vec<Arc<T>>) {
		let old = old.lock();
		let new = new.lock();

		let mut added = Vec::new();
		let mut removed = Vec::new();

		for (id, entry) in new.iter() {
			if let Some(entry) = entry.upgrade() {
				if !old.contains_key(id) {
					added.push(entry);
				}
			}
		}
		for (id, entry) in old.iter() {
			if let Some(entry) = entry.upgrade() {
				if !new.contains_key(id) {
					removed.push(entry);
				}
			}
		}
		(added, removed)
	}
	pub fn get_valid_contents(&self) -> Vec<Arc<T>> {
		self.lock()
			.iter()
			.filter_map(|pair| pair.1.upgrade())
			.collect()
	}
	pub fn set(&self, other: &Registry<T>) {
		self.lock().clone_from(&other.lock());
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
	pub fn retain<F: Fn(&Arc<T>) -> bool>(&self, f: F) {
		self.lock().retain(|_, v| {
			let Some(v) = v.upgrade() else {
				return true;
			};
			(f)(&v)
		})
	}
	pub fn remove(&self, t: &T) {
		self.lock()
			.remove(&(ptr::addr_of!(*t) as *const () as usize));
	}
	pub fn clear(&self) {
		self.lock().clear();
	}
	pub fn is_empty(&self) -> bool {
		let registry = self.0.lock();
		let Some(registry) = &*registry else {
			return true;
		};
		if registry.is_empty() {
			return true;
		}
		registry.values().all(|v| v.strong_count() == 0)
	}
}
impl<T: Send + Sync + ?Sized> Clone for Registry<T> {
	fn clone(&self) -> Self {
		Self(Mutex::new(self.0.lock().clone()))
	}
}
impl<T: Send + Sync + ?Sized> Default for Registry<T> {
	fn default() -> Self {
		Self::new()
	}
}

pub struct OwnedRegistry<T: Send + Sync + ?Sized>(Mutex<Option<FxHashMap<usize, Arc<T>>>>);

impl<T: Send + Sync + ?Sized> OwnedRegistry<T> {
	pub const fn new() -> Self {
		OwnedRegistry(const_mutex(None))
	}
	fn lock(&self) -> MappedMutexGuard<FxHashMap<usize, Arc<T>>> {
		MutexGuard::map(self.0.lock(), |r| {
			r.get_or_insert_with(FxHashMap::default)
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
	pub fn remove(&self, t: &T) -> Option<Arc<T>>
	where
		T: Sized,
	{
		self.lock()
			.remove(&(ptr::addr_of!(*t) as *const () as usize))
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

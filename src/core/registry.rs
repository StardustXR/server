#![allow(dead_code)]

use dashmap::DashMap;
use parking_lot::{MappedMutexGuard, Mutex, MutexGuard, const_mutex};
use rustc_hash::FxHashMap;
use std::ops::Deref;
use std::ptr;
use std::sync::{Arc, LazyLock, Weak};

#[derive(Debug)]
pub struct Registry<T: Send + Sync + ?Sized>(MaybeLazy<DashMap<usize, Weak<T>>>);

impl<T: Send + Sync + ?Sized> Registry<T> {
	pub const fn new() -> Self {
		Registry(MaybeLazy::Lazy(LazyLock::new(DashMap::default)))
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
		self.0
			.insert(Arc::as_ptr(t) as *const () as usize, Arc::downgrade(t));
	}
	pub fn contains(&self, t: &T) -> bool {
		self.0
			.contains_key(&(ptr::addr_of!(*t) as *const () as usize))
	}
	pub fn get_changes(old: &Registry<T>, new: &Registry<T>) -> (Vec<Arc<T>>, Vec<Arc<T>>) {
		let mut added = Vec::new();
		let mut removed = Vec::new();

		for pair in new.0.iter() {
			let (id, entry) = pair.pair();
			if let Some(entry) = entry.upgrade()
				&& !old.0.contains_key(id)
			{
				added.push(entry);
			}
		}
		for pair in old.0.iter() {
			let (id, entry) = pair.pair();
			if let Some(entry) = entry.upgrade()
				&& !new.0.contains_key(id)
			{
				removed.push(entry);
			}
		}
		(added, removed)
	}
	pub fn get_valid_contents(&self) -> Vec<Arc<T>> {
		self.0
			.iter()
			.filter_map(|pair| pair.value().upgrade())
			.collect()
	}
	pub fn set(&self, other: &Registry<T>) {
		self.clear();
		for (key, value) in other.0.deref().clone().into_iter() {
			self.0.insert(key, value);
		}
	}
	pub fn take_valid_contents(&self) -> Vec<Arc<T>> {
		let contents = self.get_valid_contents();
		self.0.clear();
		contents
	}
	pub fn retain<F: Fn(&Arc<T>) -> bool>(&self, f: F) {
		self.0.retain(|_, v| {
			let Some(v) = v.upgrade() else {
				// why would we want to retain things we can't upgrade?
				return true;
			};
			(f)(&v)
		})
	}
	pub fn remove(&self, t: &T) {
		self.0.remove(&(ptr::addr_of!(*t) as *const () as usize));
	}
	pub fn clear(&self) {
		self.0.clear();
	}
	pub fn is_empty(&self) -> bool {
		if self.0.is_empty() {
			return true;
		}
		self.0.iter().all(|v| v.value().strong_count() == 0)
	}
}

impl<T: Send + Sync + ?Sized> Clone for Registry<T> {
	fn clone(&self) -> Self {
		Self(self.0.clone())
	}
}
impl<T: Send + Sync + ?Sized> Default for Registry<T> {
	fn default() -> Self {
		Self::new()
	}
}

impl<T: Send + Sync + Sized> FromIterator<Arc<T>> for Registry<T> {
	fn from_iter<I: IntoIterator<Item = Arc<T>>>(iter: I) -> Self {
		Registry(MaybeLazy::NonLazy(
			iter.into_iter()
				.map(|i| (Arc::as_ptr(&i) as usize, Arc::downgrade(&i)))
				.collect(),
		))
	}
}

#[derive(Debug)]
enum MaybeLazy<T> {
	Lazy(LazyLock<T>),
	NonLazy(T),
}
impl<T: Clone> Clone for MaybeLazy<T> {
	fn clone(&self) -> Self {
		match self {
			MaybeLazy::Lazy(lazy_lock) => Self::NonLazy(lazy_lock.deref().clone()),
			MaybeLazy::NonLazy(v) => Self::NonLazy(v.clone()),
		}
	}
}
impl<T> Deref for MaybeLazy<T> {
	type Target = T;

	fn deref(&self) -> &Self::Target {
		match self {
			MaybeLazy::Lazy(lazy_lock) => lazy_lock,
			MaybeLazy::NonLazy(v) => v,
		}
	}
}

pub struct OwnedRegistry<T: Send + Sync + ?Sized>(Mutex<Option<FxHashMap<usize, Arc<T>>>>);

impl<T: Send + Sync + ?Sized> OwnedRegistry<T> {
	pub const fn new() -> Self {
		OwnedRegistry(const_mutex(None))
	}
	fn lock(&self) -> MappedMutexGuard<'_, FxHashMap<usize, Arc<T>>> {
		MutexGuard::map(self.0.lock(), |r| r.get_or_insert_with(FxHashMap::default))
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

use std::{
	ops::{Deref, DerefMut},
	sync::atomic::Ordering,
};

use parking_lot::{
	lock_api::{MutexGuard, RwLockReadGuard},
	Mutex, RawMutex, RawRwLock, RwLock,
};
use portable_atomic::AtomicU8;

pub struct QueuedMutex<T: Clone> {
	mutable: Mutex<Option<T>>,
	readers: AtomicU8,
	inner: RwLock<T>,
}

impl<T: Clone + Default> Default for QueuedMutex<T> {
	fn default() -> Self {
		Self::new(T::default())
	}
}

impl<T: Clone> QueuedMutex<T> {
	pub const fn new(value: T) -> QueuedMutex<T> {
		Self {
			mutable: Mutex::new(None),
			readers: AtomicU8::new(0),
			inner: RwLock::new(value),
		}
	}
	pub fn lock(&self) -> QueuedMutexLockGuard<'_, T> {
		let mut guard = self.mutable.lock();
		if guard.is_none() {
			guard.replace(self.inner.read().clone());
		}
		QueuedMutexLockGuard { mutex: self, guard }
	}
	pub fn read_now(&self) -> QueuedMutexReadGuard<'_, T> {
		let guard = self.inner.read();
		self.readers.add(1, Ordering::Relaxed);
		QueuedMutexReadGuard {
			mutex: self,
			guard: Some(guard),
		}
	}
}

pub struct QueuedMutexLockGuard<'a, T: Clone> {
	mutex: &'a QueuedMutex<T>,
	guard: MutexGuard<'a, RawMutex, Option<T>>,
}
impl<T: Clone> Deref for QueuedMutexLockGuard<'_, T> {
	type Target = T;

	fn deref(&self) -> &Self::Target {
		match self.guard.as_ref() {
			Some(v) => v,
			None => unreachable!(),
		}
	}
}
impl<T: Clone> DerefMut for QueuedMutexLockGuard<'_, T> {
	fn deref_mut(&mut self) -> &mut Self::Target {
		match self.guard.as_mut() {
			Some(v) => v,
			None => unreachable!(),
		}
	}
}

impl<T: Clone> Drop for QueuedMutexLockGuard<'_, T> {
	fn drop(&mut self) {
		if self.mutex.readers.load(Ordering::Relaxed) != 0 {
			return;
		}
		let mut write_lock = self.mutex.inner.write();
		*write_lock = match self.guard.take() {
			Some(v) => v,
			None => unreachable!(),
		}
	}
}

pub struct QueuedMutexReadGuard<'a, T: Clone> {
	mutex: &'a QueuedMutex<T>,
	guard: Option<RwLockReadGuard<'a, RawRwLock, T>>,
}

impl<T: Clone> Deref for QueuedMutexReadGuard<'_, T> {
	type Target = T;

	fn deref(&self) -> &Self::Target {
		match self.guard.as_ref() {
			Some(v) => v,
			None => unreachable!(),
		}
	}
}

impl<T: Clone> Drop for QueuedMutexReadGuard<'_, T> {
	fn drop(&mut self) {
		drop(self.guard.take());
		if self
			.mutex
			.readers
			.fetch_sub(1, Ordering::Relaxed)
			.wrapping_sub(1)
			!= 0
		{
			return;
		}

		let mut write_lock = self.mutex.inner.write();
		if let Some(v) = self.mutex.mutable.lock().take() {
			*write_lock = v
		}
	}
}

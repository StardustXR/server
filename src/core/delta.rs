use std::ops::{Deref, DerefMut};

pub struct Delta<T> {
	value: T,
	changed: bool,
}
#[allow(dead_code)]
impl<T> Delta<T> {
	pub const fn new(value: T) -> Self {
		Delta {
			value,
			changed: false,
		}
	}
	pub fn peek_delta(&self) -> Option<&T> {
		self.changed.then_some(&self.value)
	}
	pub fn delta(&mut self) -> Option<&mut T> {
		let delta = self.changed.then_some(&mut self.value);
		self.changed = false;
		delta
	}
	pub const fn value(&self) -> &T {
		&self.value
	}
	pub fn value_mut(&mut self) -> &mut T {
		self.changed = true;
		&mut self.value
	}
}
impl<T> Deref for Delta<T> {
	type Target = T;

	fn deref(&self) -> &Self::Target {
		&self.value
	}
}
impl<T> DerefMut for Delta<T> {
	fn deref_mut(&mut self) -> &mut Self::Target {
		self.changed = true;
		&mut self.value
	}
}

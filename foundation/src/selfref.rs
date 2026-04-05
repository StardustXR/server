use std::sync::{Arc, LazyLock, Weak};

pub type SelfRef<T> = LazyLock<Arc<T>>;
pub fn self_ref<T>(weak: Weak<T>) -> SelfRef<T> {
	LazyLock::new(move || weak.upgrade().unwrap())
}

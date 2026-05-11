use tokio::task::{AbortHandle, JoinHandle};

#[derive(Debug)]
pub struct AbortOnDrop(AbortHandle);
impl From<AbortHandle> for AbortOnDrop {
	fn from(value: AbortHandle) -> Self {
		Self(value)
	}
}
impl<T> From<JoinHandle<T>> for AbortOnDrop {
	fn from(value: JoinHandle<T>) -> Self {
		Self(value.abort_handle())
	}
}

impl Drop for AbortOnDrop {
	fn drop(&mut self) {
		self.0.abort();
	}
}

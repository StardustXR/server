use std::future::Future;
use tokio::task::JoinHandle;

#[allow(unused_variables)]
pub fn new<
	F: FnOnce() -> S,
	S: AsRef<str>,
	A: Future<Output = O> + Send + 'static,
	O: Send + 'static,
>(
	name_fn: F,
	async_future: A,
) -> std::io::Result<JoinHandle<O>> {
	#[cfg(not(feature = "profile_tokio"))]
	let result = Ok(tokio::task::spawn(async_future));
	#[cfg(feature = "profile_tokio")]
	let result = tokio::task::Builder::new()
		.name(name_fn().as_ref())
		.spawn(async_future);
	result
}

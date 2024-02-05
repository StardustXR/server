use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::any::Any;
use tokio::sync::mpsc::{self, unbounded_channel};

static MAIN_DESTROY_QUEUE: Lazy<(
	mpsc::UnboundedSender<Box<dyn Any + Send + Sync>>,
	Mutex<mpsc::UnboundedReceiver<Box<dyn Any + Send + Sync>>>,
)> = Lazy::new(|| {
	let (tx, rx) = unbounded_channel();
	(tx, Mutex::new(rx))
});

pub fn add<T: Any + Sync + Send>(thing: T) {
	MAIN_DESTROY_QUEUE.0.send(Box::new(thing)).unwrap();
}

pub fn clear() {
	while let Ok(thing) = MAIN_DESTROY_QUEUE.1.lock().try_recv() {
		drop(thing)
	}
}

use parking_lot::Mutex;
use std::{any::Any, sync::LazyLock};
use tokio::sync::mpsc::{self, unbounded_channel};

type Anything = Box<dyn Any + Send + Sync>;

static MAIN_DESTROY_QUEUE: LazyLock<(
	mpsc::UnboundedSender<Anything>,
	Mutex<mpsc::UnboundedReceiver<Anything>>,
)> = LazyLock::new(|| {
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

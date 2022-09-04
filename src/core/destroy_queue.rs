use parking_lot::Mutex;
use std::any::Any;

static MAIN_DESTROY_QUEUE: Mutex<Vec<Box<dyn Any + Send + Sync>>> = Mutex::new(Vec::new());

pub fn add<T: Any + Sync + Send>(thing: T) {
	MAIN_DESTROY_QUEUE.lock().push(Box::new(thing));
}

pub fn clear() {
	MAIN_DESTROY_QUEUE.lock().clear();
}

use std::marker::PhantomData;
use waynest::{
	server::{Client, Dispatcher},
	wire::ObjectId,
};

pub struct WeakDispatcher<D: Dispatcher>(ObjectId, PhantomData<D>);
impl<D: Dispatcher> WeakDispatcher<D> {
	pub fn upgrade(&self, client: &Client) -> Option<&D> {
		client.get(&self.0).as_dispatcher::<D>().ok()
	}
}

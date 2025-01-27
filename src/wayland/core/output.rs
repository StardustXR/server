use waynest::server::{Client, Dispatcher, Object, Result};

pub use waynest::server::protocol::core::wayland::wl_output::*;

#[derive(Debug, Dispatcher, Default)]
pub struct Output;

impl WlOutput for Output {
    async fn release(&self, _object: &Object, _client: &mut Client) -> Result<()> {
        todo!()
    }
}

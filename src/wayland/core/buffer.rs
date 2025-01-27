use mint::Vector2;
pub use waynest::server::protocol::core::wayland::wl_buffer::*;
use waynest::server::{
	protocol::core::wayland::wl_shm::Format, Client, Dispatcher, Object, Result,
};

#[derive(Debug, Dispatcher)]
pub struct Buffer {
	pub offset: usize,
	pub stride: usize,
	pub size: Vector2<u32>,
	pub format: Format,
}
impl WlBuffer for Buffer {
	async fn destroy(&self, _object: &Object, _client: &mut Client) -> Result<()> {
		Ok(())
	}
}

use super::{buffer::Buffer, callback::Callback};
use crate::{
	core::registry::Registry,
	wayland::{
		util::{ClientExt, DoubleBuffer},
		MessageSink,
	},
};
use mint::Vector2;
use parking_lot::Mutex;
use std::sync::{Arc, OnceLock};
use waynest::{
	server::{
		protocol::core::wayland::{
			wl_buffer::WlBuffer, wl_callback::WlCallback, wl_output::Transform, wl_surface::*,
		},
		Client, Dispatcher, Object, Result,
	},
	wire::{Message, ObjectId},
};

pub static WL_SURFACE_REGISTRY: Registry<Surface> = Registry::new();

#[derive(Debug, Clone)]
struct SurfaceState {
	buffer: Option<Arc<Buffer>>,
	density: f32,
	clean_lock: OnceLock<()>,
}
impl Default for SurfaceState {
	fn default() -> Self {
		Self {
			buffer: Default::default(),
			density: 1.0,
			clean_lock: Default::default(),
		}
	}
}

#[derive(Debug, Dispatcher)]
pub struct Surface {
	state: Mutex<DoubleBuffer<SurfaceState>>,
	message_sink: MessageSink,
	frame_callback: Mutex<Option<Message>>,
}
impl Surface {
	pub fn new(client: &Client) -> Self {
		Surface {
			state: Default::default(),
			message_sink: client.message_sink(),
			frame_callback: Default::default(),
		}
	}
	pub fn update(&self) {
		let state_lock = self.state.lock();
		if state_lock.current().clean_lock.get().is_some() {
			// then we don't need to reupload the texture
			return;
		}
		let Some(buffer) = state_lock.current().buffer.clone() else {
			return;
		};
		// then we should reupload to the gpu
		buffer.update_tex();
		let _ = state_lock.current().clean_lock.set(());
	}
	fn mark_dirty(&self) {
		self.state.lock().pending.clean_lock = Default::default();
	}
	pub fn frame_event(&self) {
		if let Some(callback_msg) = self.frame_callback.lock().take() {
			let _ = self.message_sink.send(callback_msg);
		}
	}
	pub fn size(&self) -> Option<Vector2<u32>> {
		self.state
			.lock()
			.current()
			.buffer
			.as_ref()
			.map(|b| [b.size.x as u32, b.size.y as u32].into())
	}

	async fn release_old_buffer(&self, client: &mut Client) -> Result<()> {
		let (old_buffer, object) = {
			let lock = self.state.lock();

			let Some(old_buffer) = lock.current().buffer.clone() else {
				return Ok(());
			};
			let new_buffer = lock.pending.buffer.as_ref();
			if new_buffer.map(Arc::as_ptr) == Some(Arc::as_ptr(&old_buffer)) {
				return Ok(());
			}
			drop(lock);

			(
				old_buffer.clone(),
				client.get_object(&old_buffer.id).unwrap(),
			)
		};
		old_buffer.release(&object).send(client).await?;

		Ok(())
	}
}
impl WlSurface for Surface {
	async fn attach(
		&self,
		_object: &Object,
		client: &mut Client,
		buffer: Option<ObjectId>,
		_x: i32,
		_y: i32,
	) -> Result<()> {
		self.state.lock().pending.buffer = buffer
			.and_then(|b| client.get_object(&b))
			.and_then(|b| b.as_dispatcher::<Buffer>().ok());
		Ok(())
	}

	async fn damage(
		&self,
		_object: &Object,
		_client: &mut Client,
		_x: i32,
		_y: i32,
		_width: i32,
		_height: i32,
	) -> Result<()> {
		// should be more intelligent about this but for now just make it copy everything to gpu next frame again
		self.mark_dirty();
		Ok(())
	}

	async fn frame(
		&self,
		_object: &Object,
		client: &mut Client,
		callback_id: ObjectId,
	) -> Result<()> {
		let callback = Callback.into_object(callback_id);
		self.frame_callback
			.lock()
			.replace(Callback.done(&callback, 0));
		client.insert(callback);
		Ok(())
	}

	async fn set_opaque_region(
		&self,
		_object: &Object,
		_client: &mut Client,
		_region: Option<ObjectId>,
	) -> Result<()> {
		// nothing we can really do to repaint behind this so ignore it
		Ok(())
	}

	async fn set_input_region(
		&self,
		_object: &Object,
		_client: &mut Client,
		_region: Option<ObjectId>,
	) -> Result<()> {
		// too complicated to implement this for now so who the hell cares
		Ok(())
	}

	async fn commit(&self, _object: &Object, client: &mut Client) -> Result<()> {
		self.release_old_buffer(client).await?;
		let mut lock = self.state.lock();

		let dirty =
			lock.current().clean_lock.get().is_none() || lock.pending.clean_lock.take().is_none();
		lock.apply();

		if !dirty {
			let _ = lock.current().clean_lock.set(());
		}
		Ok(())
	}

	async fn set_buffer_transform(
		&self,
		_object: &Object,
		_client: &mut Client,
		_transform: Transform,
	) -> Result<()> {
		// we just don't have the output transform or fullscreen at all so this optimization is never needed
		Ok(())
	}

	async fn set_buffer_scale(
		&self,
		_object: &Object,
		_client: &mut Client,
		scale: i32,
	) -> Result<()> {
		self.state.lock().pending.density = scale as f32;
		Ok(())
	}

	async fn damage_buffer(
		&self,
		_object: &Object,
		_client: &mut Client,
		_x: i32,
		_y: i32,
		_width: i32,
		_height: i32,
	) -> Result<()> {
		// we should upload only chunks to the gpu and do subimage copy but that's a lot rn so we won't
		self.mark_dirty();
		Ok(())
	}

	async fn offset(&self, _object: &Object, _client: &mut Client, _x: i32, _y: i32) -> Result<()> {
		Ok(())
	}

	async fn destroy(&self, _object: &Object, _client: &mut Client) -> Result<()> {
		Ok(())
	}
}
